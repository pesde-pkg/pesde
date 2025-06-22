use crate::{
	all_packages_dirs,
	download::DownloadGraphOptions,
	engine::runtime::Engines,
	graph::{
		DependencyGraph, DependencyGraphNode, DependencyGraphNodeWithTarget,
		DependencyGraphWithTarget,
	},
	linking::generator::get_file_types,
	manifest::DependencyType,
	reporters::{DownloadsReporter, PatchesReporter},
	scripts::{execute_script, ExecuteScriptHooks, ScriptName},
	source::{
		ids::PackageId,
		traits::{GetTargetOptions, PackageRef as _, PackageSource as _},
	},
	Project, RefreshedSources, LINK_LIB_NO_FILE_FOUND, SCRIPTS_LINK_FOLDER,
};
use fs_err::tokio as fs;
use futures::TryStreamExt as _;

use std::{
	collections::{HashMap, VecDeque},
	convert::Infallible,
	ffi::OsString,
	future::{self, Future},
	num::NonZeroUsize,
	path::{Path, PathBuf},
	sync::Arc,
};
use tokio::{
	pin,
	task::{spawn_blocking, JoinSet},
};
use tracing::{instrument, Instrument as _};

/// Hooks to perform actions after certain events during download and linking.
#[allow(unused_variables)]
pub trait DownloadAndLinkHooks: Send + Sync {
	/// The error type for the hooks.
	type Error: std::error::Error + Send + Sync + 'static;

	/// Called after scripts have been downloaded. The `downloaded_graph`
	/// contains all downloaded packages.
	fn on_scripts_downloaded(
		&self,
		graph: &DependencyGraphWithTarget,
	) -> impl Future<Output = Result<(), Self::Error>> + Send {
		future::ready(Ok(()))
	}

	/// Called after binary dependencies have been downloaded. The
	/// `downloaded_graph` contains all downloaded packages.
	fn on_bins_downloaded(
		&self,
		graph: &DependencyGraphWithTarget,
	) -> impl Future<Output = Result<(), Self::Error>> + Send {
		future::ready(Ok(()))
	}

	/// Called after all dependencies have been downloaded. The
	/// `downloaded_graph` contains all downloaded packages.
	fn on_all_downloaded(
		&self,
		graph: &DependencyGraphWithTarget,
	) -> impl Future<Output = Result<(), Self::Error>> + Send {
		future::ready(Ok(()))
	}
}

impl DownloadAndLinkHooks for () {
	type Error = Infallible;
}

/// Options for which dependencies to install.
#[derive(Debug, Clone, Copy)]
pub enum InstallDependenciesMode {
	/// Install all dependencies
	All,
	/// Install all dependencies, then remove [DependencyType::Dev] dependencies
	Prod,
	/// Only install dependencies which are [DependencyType::Dev]
	Dev,
}

impl InstallDependenciesMode {
	fn fits(self, dep_ty: DependencyType) -> bool {
		match (self, dep_ty) {
			(InstallDependenciesMode::Prod, DependencyType::Dev) => false,
			(InstallDependenciesMode::Dev, dep_ty) => dep_ty == DependencyType::Dev,

			_ => true,
		}
	}
}

/// Options for downloading and linking.
#[derive(Debug)]
pub struct DownloadAndLinkOptions<Reporter = (), Hooks = ()> {
	/// The reqwest client.
	pub reqwest: reqwest::Client,
	/// The downloads reporter.
	pub reporter: Option<Arc<Reporter>>,
	/// The download and link hooks.
	pub hooks: Option<Arc<Hooks>>,
	/// The refreshed sources.
	pub refreshed_sources: RefreshedSources,
	/// Which dependencies to install.
	pub install_dependencies_mode: InstallDependenciesMode,
	/// The max number of concurrent network requests.
	pub network_concurrency: NonZeroUsize,
	/// Whether to re-install all dependencies even if they are already installed
	pub force: bool,
	/// The engines this project is using
	pub engines: Arc<Engines>,
}

impl<Reporter, Hooks> DownloadAndLinkOptions<Reporter, Hooks>
where
	Reporter: DownloadsReporter + PatchesReporter + Send + Sync + 'static,
	Hooks: DownloadAndLinkHooks + Send + Sync + 'static,
{
	/// Creates a new download options with the given reqwest client and reporter.
	#[must_use]
	pub fn new(reqwest: reqwest::Client) -> Self {
		Self {
			reqwest,
			reporter: None,
			hooks: None,
			refreshed_sources: Default::default(),
			install_dependencies_mode: InstallDependenciesMode::All,
			network_concurrency: NonZeroUsize::new(16).unwrap(),
			force: false,
			engines: Default::default(),
		}
	}

	/// Sets the downloads reporter.
	#[must_use]
	pub fn reporter(mut self, reporter: impl Into<Arc<Reporter>>) -> Self {
		self.reporter.replace(reporter.into());
		self
	}

	/// Sets the download and link hooks.
	#[must_use]
	pub fn hooks(mut self, hooks: impl Into<Arc<Hooks>>) -> Self {
		self.hooks.replace(hooks.into());
		self
	}

	/// Sets the refreshed sources.
	#[must_use]
	pub fn refreshed_sources(mut self, refreshed_sources: RefreshedSources) -> Self {
		self.refreshed_sources = refreshed_sources;
		self
	}

	/// Sets which dependencies to install
	#[must_use]
	pub fn install_dependencies_mode(
		mut self,
		install_dependencies: InstallDependenciesMode,
	) -> Self {
		self.install_dependencies_mode = install_dependencies;
		self
	}

	/// Sets the max number of concurrent network requests.
	#[must_use]
	pub fn network_concurrency(mut self, network_concurrency: NonZeroUsize) -> Self {
		self.network_concurrency = network_concurrency;
		self
	}

	/// Sets whether to re-install all dependencies even if they are already installed
	#[must_use]
	pub fn force(mut self, force: bool) -> Self {
		self.force = force;
		self
	}

	/// Sets the engines this project is using
	#[must_use]
	pub fn engines(mut self, engines: impl Into<Arc<Engines>>) -> Self {
		self.engines = engines.into();
		self
	}
}

impl Clone for DownloadAndLinkOptions {
	fn clone(&self) -> Self {
		Self {
			reqwest: self.reqwest.clone(),
			reporter: self.reporter.clone(),
			hooks: self.hooks.clone(),
			refreshed_sources: self.refreshed_sources.clone(),
			install_dependencies_mode: self.install_dependencies_mode,
			network_concurrency: self.network_concurrency,
			force: self.force,
			engines: self.engines.clone(),
		}
	}
}

impl Project {
	/// Downloads a graph of dependencies and links them in the correct order
	#[instrument(
		skip_all,
		fields(install_dependencies = debug(options.install_dependencies_mode)),
		level = "debug"
	)]
	pub async fn download_and_link<Reporter, Hooks>(
		&self,
		graph: &DependencyGraph,
		options: DownloadAndLinkOptions<Reporter, Hooks>,
	) -> Result<DependencyGraphWithTarget, errors::DownloadAndLinkError<Hooks::Error>>
	where
		Reporter: DownloadsReporter + PatchesReporter + 'static,
		Hooks: DownloadAndLinkHooks + 'static,
	{
		let DownloadAndLinkOptions {
			reqwest,
			reporter,
			hooks,
			refreshed_sources,
			install_dependencies_mode,
			network_concurrency,
			force,
			engines,
		} = options;

		let reqwest = reqwest.clone();
		let manifest = self.deser_manifest().await?;

		if force {
			async fn remove_dir(dir: PathBuf) -> std::io::Result<()> {
				tracing::debug!("force deleting the `{}` folder", dir.display());

				match fs::remove_dir_all(dir).await {
					Ok(()) => Ok(()),
					Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
					Err(e) => Err(e),
				}
			}

			let mut tasks = all_packages_dirs()
				.into_iter()
				.map(|folder| remove_dir(self.package_dir().join(&folder)))
				.chain(std::iter::once(remove_dir(
					self.package_dir().join(SCRIPTS_LINK_FOLDER),
				)))
				.collect::<JoinSet<_>>();

			while let Some(task) = tasks.join_next().await {
				task.unwrap()?;
			}
		}

		// step 1. download dependencies
		let graph_to_download = {
			let mut download_graph_options = DownloadGraphOptions::<Reporter>::new(reqwest.clone())
				.refreshed_sources(refreshed_sources.clone())
				.network_concurrency(network_concurrency);

			if let Some(reporter) = reporter.clone() {
				download_graph_options = download_graph_options.reporter(reporter);
			}

			let correct_deps = if matches!(install_dependencies_mode, InstallDependenciesMode::All)
			{
				graph.clone()
			} else {
				let mut queue = graph
					.iter()
					.filter(|(_, node)| {
						node.direct
							.as_ref()
							.is_some_and(|(_, _, ty)| install_dependencies_mode.fits(*ty))
					})
					.collect::<VecDeque<_>>();

				let mut correct_deps = DependencyGraph::new();
				while let Some((id, node)) = queue.pop_front() {
					if correct_deps.insert(id.clone(), node.clone()).is_some() {
						// prevent an infinite loop with recursive dependencies
						continue;
					}

					node.dependencies
						.values()
						.filter_map(|(id, _)| graph.get(id).map(|node| (id, node)))
						.for_each(|x| queue.push_back(x));
				}

				correct_deps
			};

			let mut downloaded_graph = HashMap::new();

			let graph_to_download = if force {
				correct_deps
			} else {
				let mut tasks = correct_deps
					.into_iter()
					.map(|(id, node)| {
						let container_folder: Arc<Path> = node
							.container_folder_from_project(&id, self, manifest.target.kind())
							.into();

						async move {
							return (
								// force local packages to be updated
								if node.pkg_ref.is_local() {
									None
								} else {
									fs::metadata(&container_folder)
										.await
										.is_ok()
										.then_some(container_folder)
								},
								id,
								node,
							);
						}
					})
					.collect::<JoinSet<_>>();

				let mut graph_to_download = DependencyGraph::new();
				while let Some(task) = tasks.join_next().await {
					let (install_path, id, node) = task.unwrap();
					if let Some(install_path) = install_path {
						downloaded_graph.insert(id, (node, install_path, false));
						continue;
					}

					graph_to_download.insert(id, node);
				}

				graph_to_download
			};

			let span = tracing::debug_span!("download");
			let _guard = span.enter();

			let downloaded =
				self.download_graph(&graph_to_download, download_graph_options.clone())?;
			pin!(downloaded);

			let mut tasks = JoinSet::new();

			while let Some((id, node, fs)) = downloaded.try_next().await? {
				let container_folder =
					node.container_folder_from_project(&id, self, manifest.target.kind());

				let project = self.clone();

				tasks.spawn(async move {
					fs::create_dir_all(&container_folder).await?;

					fs.write_to(&container_folder, project.cas_dir(), true)
						.await
						.map_err(errors::DownloadAndLinkError::<Hooks::Error>::from)?;

					Ok::<_, errors::DownloadAndLinkError<Hooks::Error>>((
						id,
						(node, container_folder.into(), false),
					))
				});
			}

			while let Some(task) = tasks.join_next().await {
				let (id, data) = task.unwrap()?;
				downloaded_graph.insert(id, data);
			}

			downloaded_graph
		};

		let graph_download_data = graph_to_download
			.iter()
			.map(|(id, (_, install_path, _))| (id.clone(), install_path.clone()))
			.collect::<HashMap<_, _>>();

		let (wally_graph_to_download, other_graph_to_download) =
			graph_to_download
				.into_iter()
				.partition::<HashMap<_, _>, _>(|(_, (node, _, _))| node.pkg_ref.is_wally_package());

		let mut graph = DependencyGraphWithTarget::new();

		async fn get_graph_targets<Hooks: DownloadAndLinkHooks>(
			graph: &mut DependencyGraphWithTarget,
			project: &Project,
			downloaded_graph: HashMap<PackageId, (DependencyGraphNode, Arc<Path>, bool)>,
			engines: &Arc<Engines>,
		) -> Result<Vec<Vec<OsString>>, errors::DownloadAndLinkError<Hooks::Error>> {
			let mut tasks = downloaded_graph
				.into_iter()
				.map(|(id, (node, install_path, local))| {
					let source = node.pkg_ref.source();

					let id = Arc::new(id);
					let project = project.clone();
					let engines = engines.clone();

					async move {
						let target = source
							.get_target(
								&node.pkg_ref,
								&GetTargetOptions {
									project: project.clone(),
									path: install_path.clone(),
									id: id.clone(),
									engines: engines.clone(),
								},
							)
							.await?;

						let sync_config_args = Some(&target)
							.filter(|_| !node.pkg_ref.is_wally_package() && !local)
							.and_then(|t| t.build_files())
							.map(|build_files| {
								std::iter::once(install_path.as_os_str().to_os_string())
									.chain(build_files.iter().map(OsString::from))
									.collect::<Vec<_>>()
							});

						Ok::<_, errors::DownloadAndLinkError<Hooks::Error>>((
							Arc::into_inner(id).unwrap(),
							DependencyGraphNodeWithTarget { target, node },
							sync_config_args,
						))
					}
				})
				.collect::<JoinSet<_>>();

			let mut sync_config_args = vec![];

			while let Some(task) = tasks.join_next().await {
				let (id, node, args) = task.unwrap()?;
				graph.insert(id, node);
				if let Some(args) = args {
					sync_config_args.push(args);
				}
			}

			Ok(sync_config_args)
		}

		// step 2. get targets for non Wally packages (Wally packages require the scripts packages to be downloaded first)
		let sync_config_args =
			get_graph_targets::<Hooks>(&mut graph, self, other_graph_to_download, &engines)
				.instrument(tracing::debug_span!("get targets (non-wally)"))
				.await?;

		self.link(&graph, &manifest, &Default::default(), false)
			.instrument(tracing::debug_span!("link (non-wally)"))
			.await?;

		#[derive(Debug, Clone, Copy)]
		struct LinkingExecuteScriptHooks;

		impl ExecuteScriptHooks for LinkingExecuteScriptHooks {
			fn not_found(&self, script: ScriptName) {
				tracing::warn!(
					"not having a `{script}` script in the manifest might cause issues with linking"
				);
			}
		}

		let mut tasks = sync_config_args
			.into_iter()
			.map(|args| {
				let project = self.clone();
				let engines = engines.clone();

				async move {
					execute_script(
						ScriptName::RobloxSyncConfigGenerator,
						&project,
						&engines,
						LinkingExecuteScriptHooks,
						args,
						false,
					)
					.await
					.map(|_| ())
				}
			})
			.collect::<JoinSet<_>>();

		while let Some(result) = tasks.join_next().await {
			result.unwrap()?;
		}

		if let Some(hooks) = &hooks {
			hooks
				.on_scripts_downloaded(&graph)
				.await
				.map_err(errors::DownloadAndLinkError::Hook)?;

			hooks
				.on_bins_downloaded(&graph)
				.await
				.map_err(errors::DownloadAndLinkError::Hook)?;
		}

		// step 3. get targets for Wally packages
		get_graph_targets::<Hooks>(&mut graph, self, wally_graph_to_download, &engines)
			.instrument(tracing::debug_span!("get targets (wally)"))
			.await?;

		#[cfg(feature = "patches")]
		{
			use crate::patches::apply_patch;
			let mut tasks = manifest
				.patches
				.iter()
				.flat_map(|(name, versions)| {
					versions
						.iter()
						.map(|(v_id, path)| (PackageId::new(name.clone(), v_id.clone()), path))
				})
				.filter_map(|(id, patch_path)| graph.get(&id).map(|node| (id, node, patch_path)))
				.map(|(id, node, patch_path)| {
					let patch_path = patch_path.to_path(self.package_dir());
					let container_folder =
						node.node
							.container_folder_from_project(&id, self, manifest.target.kind());
					let reporter = reporter.clone();

					async move {
						match reporter {
							Some(reporter) => {
								apply_patch(&id, container_folder, &patch_path, reporter.clone())
									.await
							}
							None => {
								apply_patch(&id, container_folder, &patch_path, ().into()).await
							}
						}
					}
				})
				.collect::<JoinSet<_>>();

			while let Some(task) = tasks.join_next().await {
				task.unwrap()?;
			}
		}

		let mut tasks = graph_download_data
			.into_iter()
			.map(|(package_id, install_path)| {
				let span =
					tracing::info_span!("extract types", package_id = package_id.to_string());

				let node = graph[&package_id].clone();

				async move {
					let Some(lib_file) = node.target.lib_path() else {
						return Ok((package_id, vec![]));
					};

					let types = if lib_file.as_str() == LINK_LIB_NO_FILE_FOUND {
						vec![]
					} else {
						let lib_file = lib_file.to_path(&install_path);

						let contents =
							match fs::read_to_string(&lib_file).await {
								Ok(contents) => contents,
								Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
									return Err(errors::DownloadAndLinkError::<Hooks::Error>::LibFileNotFound(
									lib_file.into_boxed_path(),
								));
								}
								Err(e) => return Err(e.into()),
							};

						let types = spawn_blocking(move || get_file_types(&contents))
							.await
							.unwrap();

						tracing::debug!("contains {} exported types", types.len());

						types
					};

					Ok((package_id, types))
				}
				.instrument(span)
			})
			.collect::<JoinSet<_>>();

		let mut package_types = HashMap::<PackageId, Vec<String>>::default();

		while let Some(task) = tasks.join_next().await {
			let (id, types) = task.unwrap()?;
			package_types.insert(id, types);
		}

		// step 4. link ALL dependencies. do so with types
		self.link(&graph, &manifest, &package_types, true)
			.instrument(tracing::debug_span!("link (all)"))
			.await?;

		if let Some(hooks) = &hooks {
			hooks
				.on_all_downloaded(&graph)
				.await
				.map_err(errors::DownloadAndLinkError::Hook)?;
		}

		if matches!(install_dependencies_mode, InstallDependenciesMode::Prod) || !force {
			self.remove_unused(&graph).await?;
		}

		Ok(graph)
	}
}

/// Errors that can occur when downloading and linking dependencies
pub mod errors {
	use std::path::Path;

	use thiserror::Error;

	/// An error that can occur when downloading and linking dependencies
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum DownloadAndLinkError<E> {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// An error occurred while downloading the graph
		#[error("error downloading graph")]
		DownloadGraph(#[from] crate::download::errors::DownloadGraphError),

		/// An error occurred while linking dependencies
		#[error("error linking dependencies")]
		Linking(#[from] crate::linking::errors::LinkingError),

		/// An error occurred while executing the pesde callback
		#[error("error executing hook")]
		Hook(#[source] E),

		/// IO error
		#[error("io error")]
		Io(#[from] std::io::Error),

		/// Error getting a target
		#[error("error getting target")]
		GetTarget(#[from] crate::source::errors::GetTargetError),

		/// Removing unused dependencies failed
		#[error("error removing unused dependencies")]
		RemoveUnused(#[from] crate::linking::incremental::errors::RemoveUnusedError),

		/// Patching a package failed
		#[cfg(feature = "patches")]
		#[error("error applying patch")]
		Patch(#[from] crate::patches::errors::ApplyPatchError),

		/// Executing a script failed
		#[error("error executing script")]
		ExecuteScript(#[from] crate::scripts::errors::ExecuteScriptError),

		/// The library file was not found
		#[error("library file at `{0}` not found")]
		LibFileNotFound(Box<Path>),
	}
}
