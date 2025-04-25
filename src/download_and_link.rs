use crate::{
	all_packages_dirs,
	download::DownloadGraphOptions,
	graph::{
		DependencyGraph, DependencyGraphNode, DependencyGraphNodeWithTarget,
		DependencyGraphWithTarget,
	},
	manifest::{target::TargetKind, DependencyType},
	reporters::{DownloadsReporter, PatchesReporter},
	source::{
		ids::PackageId,
		traits::{GetTargetOptions, PackageRef as _, PackageSource as _},
	},
	Project, RefreshedSources, SCRIPTS_LINK_FOLDER,
};
use fs_err::tokio as fs;
use futures::TryStreamExt as _;

use std::{
	collections::{HashMap, VecDeque},
	convert::Infallible,
	future::{self, Future},
	num::NonZeroUsize,
	path::PathBuf,
	sync::Arc,
};
use tokio::{pin, task::JoinSet};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallDependenciesMode {
	/// Install all dependencies.
	All,
	/// Install only dependencies, not dev_dependencies.
	Prod,
	/// Install only dev_dependencies.
	Dev,
}

impl InstallDependenciesMode {
	fn fits(self, dep_ty: DependencyType) -> bool {
		match (self, dep_ty) {
			(InstallDependenciesMode::All, _) => true,
			(InstallDependenciesMode::Prod, DependencyType::Standard) => true,
			(InstallDependenciesMode::Prod, DependencyType::Peer) => true,
			(InstallDependenciesMode::Prod, DependencyType::Dev) => false,
			(InstallDependenciesMode::Dev, DependencyType::Standard) => false,
			(InstallDependenciesMode::Dev, DependencyType::Peer) => false,
			(InstallDependenciesMode::Dev, DependencyType::Dev) => true,
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
		}
	}
}

impl Project {
	/// Downloads a graph of dependencies and links them in the correct order
	#[instrument(skip_all, fields(install_dependencies = debug(options.install_dependencies_mode)), level = "debug")]
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

			let mut downloaded_graph = DependencyGraph::new();

			let mut queue = graph
				.iter()
				.filter(|(_id, node)| {
					node.direct.is_some() && install_dependencies_mode.fits(node.resolved_ty)
				})
				.collect::<VecDeque<_>>();

			let mut correct_deps = DependencyGraph::new();
			while let Some((id, node)) = queue.pop_front() {
				correct_deps.insert(id.clone(), node.clone());
				node.dependencies
					.iter()
					.filter_map(|(id, _alias)| graph.get(&id).map(|node| (id, node)))
					.for_each(|x| queue.push_back(x));
			}

			let graph_to_download = if force {
				correct_deps
			} else {
				let mut tasks = correct_deps
					.into_iter()
					.map(|(id, node)| {
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

				graph_to_download
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

		let (wally_graph_to_download, other_graph_to_download) =
			graph_to_download
				.into_iter()
				.partition::<HashMap<_, _>, _>(|(_, node)| node.pkg_ref.is_wally_package());

		let mut graph = DependencyGraphWithTarget::new();

		async fn get_graph_targets<Hooks: DownloadAndLinkHooks>(
			graph: &mut DependencyGraphWithTarget,
			project: &Project,
			manifest_target_kind: TargetKind,
			downloaded_graph: HashMap<PackageId, DependencyGraphNode>,
		) -> Result<(), errors::DownloadAndLinkError<Hooks::Error>> {
			let mut tasks = downloaded_graph
				.into_iter()
				.map(|(id, node)| {
					let source = node.pkg_ref.source();
					let path = node
						.container_folder_from_project(&id, project, manifest_target_kind)
						.as_path()
						.into();
					let id = Arc::new(id);
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
							DependencyGraphNodeWithTarget { target, node },
						))
					}
				})
				.collect::<JoinSet<_>>();

			while let Some(task) = tasks.join_next().await {
				let (id, node) = task.unwrap()?;
				graph.insert(id, node);
			}

			Ok(())
		}

		// step 2. get targets for non Wally packages (Wally packages require the scripts packages to be downloaded first)
		get_graph_targets::<Hooks>(
			&mut graph,
			self,
			manifest.target.kind(),
			other_graph_to_download,
		)
		.instrument(tracing::debug_span!("get targets (non-wally)"))
		.await?;

		self.link_dependencies(&graph, false)
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
			wally_graph_to_download,
		)
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

		// step 4. link ALL dependencies. do so with types
		self.link_dependencies(&graph, true)
			.instrument(tracing::debug_span!("link (all)"))
			.await?;

		if let Some(hooks) = &hooks {
			hooks
				.on_all_downloaded(&graph)
				.await
				.map_err(errors::DownloadAndLinkError::Hook)?;
		}

		if install_dependencies_mode != InstallDependenciesMode::All || !force {
			self.remove_unused(&graph).await?;
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

		/// Removing unused dependencies failed
		#[error("error removing unused dependencies")]
		RemoveUnused(#[from] crate::linking::incremental::errors::RemoveUnusedError),

		/// Patching a package failed
		#[cfg(feature = "patches")]
		#[error("error applying patch")]
		Patch(#[from] crate::patches::errors::ApplyPatchError),
	}
}
