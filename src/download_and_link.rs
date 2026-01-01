use crate::{
	LINK_LIB_NO_FILE_FOUND, Project, RefreshedSources,
	download::DownloadGraphOptions,
	engine::runtime::Engines,
	graph::{DependencyGraph, DependencyGraphNode},
	linking::generator::get_file_types,
	manifest::{DependencyType, target::Target},
	private_dir,
	reporters::{DownloadsReporter, PatchesReporter},
	source::{
		ids::PackageId,
		traits::{GetTargetOptions, PackageSource as _},
	},
};
use fs_err::tokio as fs;
use futures::TryStreamExt as _;
use relative_path::RelativePath;

use std::{
	collections::{BTreeSet, HashMap, HashSet},
	convert::Infallible,
	future::{self, Future},
	num::NonZeroUsize,
	path::Path,
	sync::Arc,
};
use tokio::{
	pin,
	task::{JoinSet, spawn_blocking},
};
use tracing::{Instrument as _, instrument};

/// Hooks to perform actions after certain events during download and linking.
#[allow(unused_variables)]
pub trait DownloadAndLinkHooks: Send + Sync {
	/// The error type for the hooks.
	type Error: std::error::Error + Send + Sync + 'static;

	/// Called after binary dependencies have been downloaded.
	/// `aliases` contains all the aliases binaries are known by.
	fn on_bins_downloaded<'a>(
		&self,
		importer: &RelativePath,
		aliases: impl Iterator<Item = &'a str> + Send,
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
	) -> Result<HashMap<PackageId, Arc<Target>>, errors::DownloadAndLinkError<Hooks::Error>>
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

		if force {
			let mut tasks = graph
				.importers
				.keys()
				.map(|importer| {
					let dependencies_dir = private_dir(self, importer).join("dependencies");

					async move {
						match fs::remove_dir_all(&dependencies_dir).await {
							Ok(_) => Ok(()),
							Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
							Err(e) => Err(e),
						}
					}
				})
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

			let mut importer_deps = HashMap::<PackageId, HashSet<Arc<RelativePath>>>::new();
			for (importer, dependencies) in &graph.importers {
				let mut queue = dependencies
					.values()
					.filter(|(_, _, ty)| install_dependencies_mode.fits(*ty))
					.map(|(id, _, _)| id.clone())
					.collect::<Vec<_>>();
				let mut visited = HashSet::new();

				while let Some(pkg_id) = queue.pop() {
					if visited.insert(pkg_id.clone())
						&& let Some(node) = graph.nodes.get(&pkg_id)
					{
						for dep_id in node.dependencies.values() {
							queue.push(dep_id.clone());
						}
					}
				}

				for id in visited {
					importer_deps
						.entry(id)
						.or_default()
						.insert(importer.clone());
				}
			}

			let mut downloaded_packages = HashMap::<PackageId, HashSet<Arc<RelativePath>>>::new();

			let graph_to_download = if force {
				importer_deps
			} else {
				let mut tasks = importer_deps
					.into_iter()
					.flat_map(|(id, importers)| {
						importers
							.into_iter()
							.map(move |importer| (importer, id.clone()))
					})
					.map(|(importer, id)| {
						let dependency_dir = private_dir(self, &importer)
							.join("dependencies")
							.join(DependencyGraphNode::container_dir_top_level(&id));
						async move {
							if id.pkg_ref().is_local() {
								return (importer, id, false);
							}

							(importer, id, fs::metadata(&dependency_dir).await.is_ok())
						}
					})
					.collect::<JoinSet<_>>();

				let mut deps_to_download = HashMap::<PackageId, HashSet<Arc<RelativePath>>>::new();
				while let Some(task) = tasks.join_next().await {
					let (importer, id, installed) = task.unwrap();
					if installed {
						downloaded_packages.entry(id).or_default().insert(importer);
						continue;
					}

					deps_to_download.entry(id).or_default().insert(importer);
				}

				deps_to_download
			};

			let span = tracing::debug_span!("download");
			let _guard = span.enter();

			let downloaded = self.download_graph(
				graph_to_download.keys().cloned(),
				download_graph_options.clone(),
			)?;
			pin!(downloaded);

			let mut tasks = JoinSet::new();

			while let Some((id, fs)) = downloaded.try_next().await? {
				let fs = Arc::new(fs);

				for importer in &graph_to_download[&id] {
					let container_dir = private_dir(self, importer)
						.join("dependencies")
						.join(DependencyGraphNode::container_dir_top_level(&id));

					let importer = importer.clone();
					let project = self.clone();
					let id = id.clone();
					let fs = fs.clone();

					tasks.spawn(async move {
						fs::create_dir_all(&container_dir).await?;

						fs.write_to(&container_dir, project.cas_dir(), true)
							.await
							.map_err(errors::DownloadAndLinkError::<Hooks::Error>::from)?;

						Ok::<_, errors::DownloadAndLinkError<Hooks::Error>>((id, importer))
					});
				}
			}

			while let Some(task) = tasks.join_next().await {
				let (id, importer) = task.unwrap()?;
				downloaded_packages.entry(id).or_default().insert(importer);
			}

			#[cfg(feature = "patches")]
			{
				use crate::patches::apply_patch;
				let mut tasks = self
					.clone()
					.into_root_project()
					.deser_manifest()
					.await?
					.patches
					.iter()
					.filter_map(|(id, patch_path)| {
						downloaded_packages.get(id).map(|importers| {
							(
								id,
								importers,
								Arc::<Path>::from(patch_path.to_path(self.root_dir())),
							)
						})
					})
					.flat_map(|(id, importers, patch_path)| {
						importers
							.iter()
							.map(move |importer| (id.clone(), importer, patch_path.clone()))
					})
					.map(|(id, importer, patch_path)| {
						let container_dir = private_dir(self, importer)
							.join("dependencies")
							.join(DependencyGraphNode::container_dir_top_level(&id));
						let reporter = reporter.clone();

						async move {
							match reporter {
								Some(reporter) => {
									apply_patch(&id, container_dir, &patch_path, reporter.clone())
										.await
								}
								None => {
									apply_patch(&id, container_dir, &patch_path, ().into()).await
								}
							}
						}
					})
					.collect::<JoinSet<_>>();

				while let Some(task) = tasks.join_next().await {
					task.unwrap()?;
				}
			}

			downloaded_packages
		};

		let (wally_graph_to_download, other_graph_to_download) = graph_to_download
			.iter()
			.partition::<HashMap<_, _>, _>(|(id, _)| id.pkg_ref().is_wally_package());

		let mut package_targets = HashMap::new();

		let get_graph_targets =
			async |package_targets: &mut HashMap<PackageId, Arc<Target>>,
			       downloaded_graph: HashMap<&PackageId, &HashSet<Arc<RelativePath>>>| {
				let mut tasks = downloaded_graph
					.into_iter()
					.map(|(id, importers)| {
						// importer does not matter here, as it is the same package being linked in different places
						let install_path = private_dir(self, importers.iter().next().unwrap())
							.join("dependencies")
							.join(DependencyGraphNode::container_dir_top_level(id))
							.into();
						let project = self.clone();
						let engines = engines.clone();
						let id = id.clone();

						async move {
							let target = id
								.source()
								.get_target(
									id.pkg_ref(),
									&GetTargetOptions {
										project,
										path: install_path,
										version_id: id.v_id(),
										engines,
									},
								)
								.await?;

							Ok::<_, errors::DownloadAndLinkError<Hooks::Error>>((id, target))
						}
					})
					.collect::<JoinSet<_>>();

				while let Some(task) = tasks.join_next().await {
					let (id, target) = task.unwrap()?;
					package_targets.insert(id, Arc::new(target));
				}

				Ok::<_, errors::DownloadAndLinkError<Hooks::Error>>(())
			};

		// step 2. get targets for non Wally packages (Wally packages require the scripts packages to be downloaded first)
		get_graph_targets(&mut package_targets, other_graph_to_download)
			.instrument(tracing::debug_span!("get targets (non-wally)"))
			.await?;

		self.link(graph, &package_targets, &Default::default())
			.instrument(tracing::debug_span!("link (non-wally)"))
			.await?;

		if let Some(hooks) = &hooks {
			let binary_packages = package_targets
				.iter()
				.filter_map(|(id, target)| target.bin_path().is_some().then_some(id))
				.collect::<BTreeSet<_>>();

			for (importer, dependencies) in &graph.importers {
				let aliases = dependencies
					.iter()
					.filter_map(|(alias, (id, _, _))| {
						binary_packages.contains(id).then_some(alias.as_str())
					})
					// TODO: include all binary aliases from dependencies' dependencies
					.collect::<BTreeSet<_>>();

				hooks
					.on_bins_downloaded(importer, aliases.into_iter())
					.await
					.map_err(errors::DownloadAndLinkError::Hook)?;
			}
		}

		// step 3. get targets for Wally packages
		get_graph_targets(&mut package_targets, wally_graph_to_download)
			.instrument(tracing::debug_span!("get targets (wally)"))
			.await?;

		let mut tasks = package_targets
			.iter()
			.map(|(package_id, target)| {
				// importer does not matter here, as it is the same package being linked in different places
				let install_path =
					private_dir(self, graph_to_download[package_id].iter().next().unwrap())
						.join("dependencies")
						.join(DependencyGraphNode::container_dir_top_level(package_id));

				let span =
					tracing::info_span!("extract types", package_id = package_id.to_string());

				let package_id = package_id.clone();
				let lib_path = target.lib_path().map(RelativePath::to_relative_path_buf);

				async move {
					let Some(lib_file) = lib_path else {
						return Ok((package_id, vec![]));
					};

					let types = if lib_file.as_str() == LINK_LIB_NO_FILE_FOUND {
						vec![]
					} else {
						let lib_file = lib_file.to_path(install_path);

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

		let mut package_types = HashMap::<PackageId, Arc<[String]>>::default();

		while let Some(task) = tasks.join_next().await {
			let (id, types) = task.unwrap()?;
			package_types.insert(id, types.into());
		}

		// step 4. link ALL dependencies. do so with types
		self.link(graph, &package_targets, &package_types)
			.instrument(tracing::debug_span!("link (all)"))
			.await?;

		if matches!(install_dependencies_mode, InstallDependenciesMode::Prod) || !force {
			self.remove_unused(graph).await?;
		}

		Ok(package_targets)
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

		/// The library file was not found
		#[error("library file at `{0}` not found")]
		LibFileNotFound(Box<Path>),
	}
}
