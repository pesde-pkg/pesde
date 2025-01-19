use crate::{
	download::DownloadGraphOptions,
	graph::{
		DependencyGraph, DependencyGraphNode, DependencyGraphNodeWithTarget,
		DependencyGraphWithTarget,
	},
	manifest::{target::TargetKind, Alias, DependencyType},
	reporters::DownloadsReporter,
	source::{
		ids::PackageId,
		traits::{GetTargetOptions, PackageRef, PackageSource},
	},
	Project, RefreshedSources, PACKAGES_CONTAINER_NAME, SCRIPTS_LINK_FOLDER,
};
use fs_err::tokio as fs;
use futures::{FutureExt, TryStreamExt};
use std::{
	collections::{HashMap, HashSet},
	convert::Infallible,
	future::{self, Future},
	num::NonZeroUsize,
	path::{Path, PathBuf},
	sync::Arc,
};
use tokio::{pin, task::JoinSet};
use tracing::{instrument, Instrument};

/// Hooks to perform actions after certain events during download and linking.
#[allow(unused_variables)]
pub trait DownloadAndLinkHooks {
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
	/// Whether to skip dev dependencies.
	pub prod: bool,
	/// The max number of concurrent network requests.
	pub network_concurrency: NonZeroUsize,
	/// Whether to re-install all dependencies even if they are already installed
	pub force: bool,
}

impl<Reporter, Hooks> DownloadAndLinkOptions<Reporter, Hooks>
where
	Reporter: DownloadsReporter + Send + Sync + 'static,
	Hooks: DownloadAndLinkHooks + Send + Sync + 'static,
{
	/// Creates a new download options with the given reqwest client and reporter.
	pub fn new(reqwest: reqwest::Client) -> Self {
		Self {
			reqwest,
			reporter: None,
			hooks: None,
			refreshed_sources: Default::default(),
			prod: false,
			network_concurrency: NonZeroUsize::new(16).unwrap(),
			force: false,
		}
	}

	/// Sets the downloads reporter.
	pub fn reporter(mut self, reporter: impl Into<Arc<Reporter>>) -> Self {
		self.reporter.replace(reporter.into());
		self
	}

	/// Sets the download and link hooks.
	pub fn hooks(mut self, hooks: impl Into<Arc<Hooks>>) -> Self {
		self.hooks.replace(hooks.into());
		self
	}

	/// Sets the refreshed sources.
	pub fn refreshed_sources(mut self, refreshed_sources: RefreshedSources) -> Self {
		self.refreshed_sources = refreshed_sources;
		self
	}

	/// Sets whether to skip dev dependencies.
	pub fn prod(mut self, prod: bool) -> Self {
		self.prod = prod;
		self
	}

	/// Sets the max number of concurrent network requests.
	pub fn network_concurrency(mut self, network_concurrency: NonZeroUsize) -> Self {
		self.network_concurrency = network_concurrency;
		self
	}

	/// Sets whether to re-install all dependencies even if they are already installed
	pub fn force(mut self, force: bool) -> Self {
		self.force = force;
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
			prod: self.prod,
			network_concurrency: self.network_concurrency,
			force: self.force,
		}
	}
}

fn all_packages_dirs() -> HashSet<String> {
	let mut dirs = HashSet::new();
	for target_kind_a in TargetKind::VARIANTS {
		for target_kind_b in TargetKind::VARIANTS {
			dirs.insert(target_kind_a.packages_folder(*target_kind_b));
		}
	}
	dirs
}

impl Project {
	/// Downloads a graph of dependencies and links them in the correct order
	#[instrument(skip_all, fields(prod = options.prod), level = "debug")]
	pub async fn download_and_link<Reporter, Hooks>(
		&self,
		graph: &Arc<DependencyGraph>,
		options: DownloadAndLinkOptions<Reporter, Hooks>,
	) -> Result<DependencyGraphWithTarget, errors::DownloadAndLinkError<Hooks::Error>>
	where
		Reporter: DownloadsReporter + 'static,
		Hooks: DownloadAndLinkHooks + 'static,
	{
		let DownloadAndLinkOptions {
			reqwest,
			reporter,
			hooks,
			refreshed_sources,
			prod,
			network_concurrency,
			force,
		} = options;

		let graph = graph.clone();
		let reqwest = reqwest.clone();
		let manifest = self.deser_manifest().await?;

		if force {
			let mut deleted_folders = HashMap::new();

			async fn remove_dir(package_dir: PathBuf, folder: String) -> std::io::Result<()> {
				tracing::debug!("force deleting the {folder} folder");

				match fs::remove_dir_all(package_dir.join(&folder)).await {
					Ok(()) => Ok(()),
					Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
					Err(e) => Err(e),
				}
			}

			for folder in all_packages_dirs() {
				let package_dir = self.package_dir().to_path_buf();

				deleted_folders
					.entry(folder.to_string())
					.or_insert_with(|| remove_dir(package_dir, folder));
			}

			deleted_folders.insert(
				SCRIPTS_LINK_FOLDER.to_string(),
				remove_dir(
					self.package_dir().to_path_buf(),
					SCRIPTS_LINK_FOLDER.to_string(),
				),
			);

			let mut tasks = deleted_folders.into_values().collect::<JoinSet<_>>();
			while let Some(task) = tasks.join_next().await {
				task.unwrap()?;
			}
		}

		// step 1. download dependencies
		let downloaded_graph = {
			let mut download_graph_options = DownloadGraphOptions::<Reporter>::new(reqwest.clone())
				.refreshed_sources(refreshed_sources.clone())
				.network_concurrency(network_concurrency);

			if let Some(reporter) = reporter {
				download_graph_options = download_graph_options.reporter(reporter.clone());
			}

			let mut downloaded_graph = DependencyGraph::new();

			let graph_to_download = if force {
				graph.clone()
			} else {
				let mut tasks = graph
					.iter()
					.map(|(id, node)| {
						let id = id.clone();
						let node = node.clone();
						let container_folder =
							node.container_folder_from_project(&id, self, manifest.target.kind());

						async move {
							return (id, node, fs::metadata(&container_folder).await.is_ok());
						}
					})
					.collect::<JoinSet<_>>();

				let mut graph_to_download = DependencyGraph::new();
				while let Some(task) = tasks.join_next().await {
					let (id, node, installed) = task.unwrap();
					if installed {
						downloaded_graph.insert(id, node);
						continue;
					}

					graph_to_download.insert(id, node);
				}

				Arc::new(graph_to_download)
			};

			let downloaded = self
				.download_graph(&graph_to_download, download_graph_options.clone())
				.instrument(tracing::debug_span!("download"))
				.await?;
			pin!(downloaded);

			let mut tasks = JoinSet::new();

			while let Some((id, node, fs)) = downloaded.try_next().await? {
				let container_folder =
					node.container_folder_from_project(&id, self, manifest.target.kind());

				downloaded_graph.insert(id, node);

				let cas_dir = self.cas_dir().to_path_buf();
				tasks.spawn(async move {
					fs::create_dir_all(&container_folder).await?;
					fs.write_to(container_folder, cas_dir, true).await
				});
			}

			while let Some(task) = tasks.join_next().await {
				task.unwrap()?;
			}

			downloaded_graph
		};

		let (downloaded_wally_graph, downloaded_other_graph) = downloaded_graph
			.into_iter()
			.partition::<HashMap<_, _>, _>(|(_, node)| node.pkg_ref.is_wally_package());

		let mut graph = Arc::new(DependencyGraphWithTarget::new());

		async fn get_graph_targets<Hooks: DownloadAndLinkHooks>(
			graph: &mut Arc<DependencyGraphWithTarget>,
			project: &Project,
			manifest_target_kind: TargetKind,
			downloaded_graph: HashMap<PackageId, DependencyGraphNode>,
		) -> Result<(), errors::DownloadAndLinkError<Hooks::Error>> {
			let mut tasks = downloaded_graph
				.into_iter()
				.map(|(id, node)| {
					let source = node.pkg_ref.source();
					let path = Arc::from(
						node.container_folder_from_project(&id, project, manifest_target_kind)
							.as_path(),
					);
					let id = Arc::new(id.clone());
					let project = project.clone();

					async move {
						let target = source
							.get_target(
								&node.pkg_ref,
								&GetTargetOptions {
									project,
									path,
									id: id.clone(),
								},
							)
							.await?;

						Ok::<_, errors::DownloadAndLinkError<Hooks::Error>>((
							Arc::into_inner(id).unwrap(),
							DependencyGraphNodeWithTarget { node, target },
						))
					}
				})
				.collect::<JoinSet<_>>();

			while let Some(task) = tasks.join_next().await {
				let (id, node) = task.unwrap()?;
				Arc::get_mut(graph).unwrap().insert(id, node);
			}

			Ok(())
		}

		// step 2. get targets for non Wally packages (Wally packages require the scripts packages to be downloaded first)
		get_graph_targets::<Hooks>(
			&mut graph,
			self,
			manifest.target.kind(),
			downloaded_other_graph,
		)
		.instrument(tracing::debug_span!("get targets (non-wally)"))
		.await?;

		self.link_dependencies(graph.clone(), false)
			.instrument(tracing::debug_span!("link (non-wally)"))
			.await?;

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
		get_graph_targets::<Hooks>(
			&mut graph,
			self,
			manifest.target.kind(),
			downloaded_wally_graph,
		)
		.instrument(tracing::debug_span!("get targets (wally)"))
		.await?;

		// step 4. link ALL dependencies. do so with types
		self.link_dependencies(graph.clone(), true)
			.instrument(tracing::debug_span!("link (all)"))
			.await?;

		if let Some(hooks) = &hooks {
			hooks
				.on_all_downloaded(&graph)
				.await
				.map_err(errors::DownloadAndLinkError::Hook)?;
		}

		let mut graph = Arc::into_inner(graph).unwrap();
		let manifest = Arc::new(manifest);

		if prod {
			let (dev_graph, prod_graph) = graph
				.into_iter()
				.partition::<DependencyGraphWithTarget, _>(|(_, node)| {
					node.node.resolved_ty == DependencyType::Dev
				});

			graph = prod_graph;
			let dev_graph = Arc::new(dev_graph);

			// the `true` argument means it'll remove the dependencies linkers
			self.link(
				&dev_graph,
				&manifest,
				&Arc::new(Default::default()),
				false,
				true,
			)
			.await?;
		}

		if !force {
			async fn remove_empty_dir(path: &Path) -> std::io::Result<()> {
				match fs::remove_dir(path).await {
					Ok(()) => Ok(()),
					Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
					Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
					Err(e) => Err(e),
				}
			}

			fn index_entry(
				entry: fs::DirEntry,
				packages_index_dir: &Path,
				tasks: &mut JoinSet<std::io::Result<()>>,
				used_paths: &Arc<HashSet<PathBuf>>,
				#[cfg(feature = "wally-compat")] used_wally_paths: &Arc<HashSet<PathBuf>>,
			) {
				let path = entry.path();
				let path_relative = path.strip_prefix(packages_index_dir).unwrap().to_path_buf();

				let is_wally = entry
					.file_name()
					.to_str()
					.expect("non UTF-8 folder name in packages index")
					.contains("@");
				if is_wally {
					#[cfg(feature = "wally-compat")]
					if !used_wally_paths.contains(&path_relative) {
						tasks.spawn(async { fs::remove_dir_all(path).await });
					}

					#[cfg(not(feature = "wally-compat"))]
					{
						tracing::error!(
							"found Wally package in index despite feature being disabled at `{}`",
							path.display()
						);
					}

					return ();
				}

				let used_paths = used_paths.clone();
				tasks.spawn(async move {
					let mut tasks = JoinSet::new();

					let mut entries = fs::read_dir(&path).await?;
					while let Some(entry) = entries.next_entry().await? {
						let version = entry.file_name();
						let path_relative = path_relative.join(&version);

						if used_paths.contains(&path_relative) {
							continue;
						}

						let path = entry.path();
						tasks.spawn(async { fs::remove_dir_all(path).await });
					}

					while let Some(task) = tasks.join_next().await {
						task.unwrap()?;
					}

					remove_empty_dir(&path).await
				});
			}

			fn packages_entry(
				entry: fs::DirEntry,
				tasks: &mut JoinSet<std::io::Result<()>>,
				expected_aliases: &Arc<HashSet<Alias>>,
			) {
				let expected_aliases = expected_aliases.clone();
				tasks.spawn(async move {
					if !entry.file_type().await?.is_file() {
						return Ok(());
					}

					let path = entry.path();
					let name = path
						.file_stem()
						.unwrap()
						.to_str()
						.expect("non UTF-8 file name in packages folder");
					let name = name.strip_suffix(".bin").unwrap_or(name);
					let name = match name.parse::<Alias>() {
						Ok(name) => name,
						Err(e) => {
							tracing::error!("invalid alias in packages folder: {e}");
							return Ok(());
						}
					};

					if !expected_aliases.contains(&name) {
						fs::remove_file(path).await?;
					}

					Ok(())
				});
			}

			let used_paths = graph
				.iter()
				.filter(|(_, node)| !node.node.pkg_ref.is_wally_package())
				.map(|(id, node)| {
					node.node
						.container_folder(id)
						.version_folder()
						.to_path_buf()
				})
				.collect::<HashSet<_>>();
			let used_paths = Arc::new(used_paths);
			#[cfg(feature = "wally-compat")]
			let used_wally_paths = graph
				.iter()
				.filter(|(_, node)| node.node.pkg_ref.is_wally_package())
				.map(|(id, node)| {
					node.node
						.container_folder(id)
						.version_folder()
						.to_path_buf()
				})
				.collect::<HashSet<_>>();
			#[cfg(feature = "wally-compat")]
			let used_wally_paths = Arc::new(used_wally_paths);

			let mut tasks = all_packages_dirs()
				.into_iter()
				.map(|folder| {
					let packages_dir = self.package_dir().join(&folder);
					let packages_index_dir = packages_dir.join(PACKAGES_CONTAINER_NAME);
					let used_paths = used_paths.clone();
					#[cfg(feature = "wally-compat")]
					let used_wally_paths = used_wally_paths.clone();

					let expected_aliases = graph
						.iter()
						.filter(|(id, _)| {
							manifest
								.target
								.kind()
								.packages_folder(id.version_id().target())
								== folder
						})
						.filter_map(|(_, node)| {
							node.node.direct.as_ref().map(|(alias, _, _)| alias.clone())
						})
						.collect::<HashSet<_>>();
					let expected_aliases = Arc::new(expected_aliases);

					async move {
						let mut index_entries = match fs::read_dir(&packages_index_dir).await {
							Ok(entries) => entries,
							Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
							Err(e) => return Err(e),
						};
						// we don't handle NotFound here because the upper level will handle it
						let mut packages_entries = fs::read_dir(&packages_dir).await?;
						let mut tasks = JoinSet::new();

						loop {
							tokio::select! {
								Some(entry) = index_entries.next_entry().map(Result::transpose) => {
									index_entry(
										entry?,
										&packages_index_dir,
										&mut tasks,
										&used_paths,
										#[cfg(feature = "wally-compat")]
										&used_wally_paths,
									);
								}
								Some(entry) = packages_entries.next_entry().map(Result::transpose) => {
									packages_entry(
										entry?,
										&mut tasks,
										&expected_aliases,
									);
								}
								else => break,
							}
						}

						while let Some(task) = tasks.join_next().await {
							task.unwrap()?;
						}

						remove_empty_dir(&packages_index_dir).await?;
						remove_empty_dir(&packages_dir).await?;

						Ok::<_, std::io::Error>(())
					}
				})
				.collect::<JoinSet<_>>();

			while let Some(task) = tasks.join_next().await {
				task.unwrap()?;
			}
		}

		Ok(graph)
	}
}

/// Errors that can occur when downloading and linking dependencies
pub mod errors {
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
	}
}
