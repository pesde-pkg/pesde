use crate::Importer;
use crate::LINK_LIB_NO_FILE_FOUND;
use crate::PACKAGES_CONTAINER_NAME;
use crate::Project;
use crate::RefreshedSources;
use crate::download::DownloadGraphOptions;
use crate::linking::generator::get_file_types;
use crate::manifest::DependencyType;
use crate::reporters::DownloadsReporter;
use crate::reporters::PatchesReporter;
use crate::resolver::DependencyGraph;
use crate::resolver::DependencyGraphNode;
use crate::source::RealmExt as _;
use crate::source::ids::PackageId;
use crate::source::traits::GetExportsOptions;
use crate::source::traits::PackageExports;
use crate::source::traits::PackageSource as _;
use fs_err::tokio as fs;
use futures::TryStreamExt as _;

use std::collections::HashMap;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use tokio::pin;
use tokio::task::JoinSet;
use tokio::task::spawn_blocking;
use tracing::Instrument as _;
use tracing::instrument;

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
pub struct DownloadAndLinkOptions<Reporter = ()> {
	/// The reqwest client.
	pub reqwest: reqwest::Client,
	/// The downloads reporter.
	pub reporter: Option<Arc<Reporter>>,
	/// The refreshed sources.
	pub refreshed_sources: RefreshedSources,
	/// Which dependencies to install.
	pub install_dependencies_mode: InstallDependenciesMode,
	/// The max number of concurrent network requests.
	pub network_concurrency: NonZeroUsize,
	/// Whether to re-install all dependencies even if they are already installed
	pub force: bool,
}

impl<Reporter> DownloadAndLinkOptions<Reporter>
where
	Reporter: DownloadsReporter + PatchesReporter + Send + Sync + 'static,
{
	/// Creates a new download options with the given reqwest client and reporter.
	#[must_use]
	pub fn new(reqwest: reqwest::Client) -> Self {
		Self {
			reqwest,
			reporter: None,
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
			refreshed_sources: self.refreshed_sources.clone(),
			install_dependencies_mode: self.install_dependencies_mode,
			network_concurrency: self.network_concurrency,
			force: self.force,
		}
	}
}

impl Project {
	/// Downloads a graph of dependencies and links them in the correct order
	/// Fills the DependencyGraphNode::checksum field
	#[instrument(
		skip_all,
		fields(install_dependencies = debug(options.install_dependencies_mode)),
		level = "debug"
	)]
	pub async fn download_and_link<Reporter>(
		&self,
		graph: &DependencyGraph,
		options: DownloadAndLinkOptions<Reporter>,
	) -> Result<HashMap<PackageId, Arc<PackageExports>>, errors::DownloadAndLinkError>
	where
		Reporter: DownloadsReporter + PatchesReporter + 'static,
	{
		let DownloadAndLinkOptions {
			reqwest,
			reporter,
			refreshed_sources,
			install_dependencies_mode,
			network_concurrency,
			force,
		} = options;

		let reqwest = reqwest.clone();

		if force {
			let mut tasks = graph
				.importers
				.keys()
				.map(|importer| {
					let subproject = self.clone().subproject(importer.clone());

					async move {
						match fs::remove_dir_all(subproject.dependencies_dir()).await {
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

			let mut importer_deps = HashMap::<PackageId, HashSet<Importer>>::new();
			let mut visited = HashSet::new();
			let mut queue = vec![];
			for (importer, graph_importer) in &graph.importers {
				queue.extend(
					graph_importer
						.dependencies
						.values()
						.filter(|(_, _, ty)| install_dependencies_mode.fits(*ty))
						.map(|(id, _, _)| id.clone()),
				);

				while let Some(pkg_id) = queue.pop() {
					if let Some(node) = graph.nodes.get(&pkg_id)
						&& visited.insert(pkg_id)
					{
						for dep in node.dependencies.values() {
							// don't need to download dev dependencies of dependencies
							if dep.ty == DependencyType::Dev {
								continue;
							}
							queue.push(dep.id.clone());
						}
					}
				}

				for id in visited.drain() {
					importer_deps
						.entry(id)
						.or_default()
						.insert(importer.clone());
				}
			}

			let mut downloaded_packages = HashMap::<PackageId, HashSet<Importer>>::new();

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
						let subproject = self.clone().subproject(importer.clone());
						let container_dir = subproject
							.dependencies_dir()
							.join(graph.realm_of(&importer, &id).packages_dir())
							.join(PACKAGES_CONTAINER_NAME)
							.join(DependencyGraphNode::container_dir(
								&id,
								&graph.nodes[&id].structure_kind,
							));
						async move {
							if id.pkg_ref().is_local() {
								return (importer, id, false);
							}

							(importer, id, fs::metadata(&container_dir).await.is_ok())
						}
					})
					.collect::<JoinSet<_>>();

				let mut deps_to_download = HashMap::<PackageId, HashSet<Importer>>::new();
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

			// mutable references right below, need to collect to satisfy the borrow checker
			#[allow(clippy::needless_collect)]
			let downloaded = self.download_graph(
				graph_to_download
					.keys()
					.map(|id| (id.clone(), graph.nodes[id].structure_kind.clone()))
					.collect::<Vec<_>>(),
				download_graph_options.clone(),
			)?;
			pin!(downloaded);

			let mut tasks = JoinSet::new();

			while let Some((id, fs)) = downloaded.try_next().await? {
				let fs = Arc::new(fs);

				for importer in &graph_to_download[&id] {
					let subproject = self.clone().subproject(importer.clone());

					let container_dir = subproject
						.dependencies_dir()
						.join(graph.realm_of(importer, &id).packages_dir())
						.join(PACKAGES_CONTAINER_NAME)
						.join(DependencyGraphNode::container_dir(
							&id,
							&graph.nodes[&id].structure_kind,
						));

					let id = id.clone();
					let fs = fs.clone();

					tasks.spawn(async move {
						fs::create_dir_all(&container_dir).await?;

						fs.write_to(&container_dir, subproject.project().cas_dir(), true)
							.await
							.map_err(errors::DownloadAndLinkError::from)?;

						Ok::<_, errors::DownloadAndLinkError>((id, subproject.importer().clone()))
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
					.subproject(Importer::root())
					.deser_manifest()
					.await?
					.workspace
					.patches
					.iter()
					.filter_map(|(id, patch_path)| {
						downloaded_packages.get(id).map(|importers| {
							(
								id,
								importers,
								Arc::<Path>::from(patch_path.to_path(self.dir())),
							)
						})
					})
					.flat_map(|(id, importers, patch_path)| {
						importers
							.iter()
							.map(move |importer| (id.clone(), importer, patch_path.clone()))
					})
					.map(|(id, importer, patch_path)| {
						let subproject = self.clone().subproject(importer.clone());
						let reporter = reporter.clone();

						let container_dir = subproject
							.dependencies_dir()
							.join(graph.realm_of(importer, &id).packages_dir())
							.join(PACKAGES_CONTAINER_NAME)
							.join(DependencyGraphNode::container_dir(
								&id,
								&graph.nodes[&id].structure_kind,
							));

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
			.partition::<HashMap<_, _>, _>(|(id, _)| graph.nodes[id].structure_kind.is_wally());

		let mut package_exports = HashMap::new();

		let get_graph_exports =
			async |package_exports: &mut HashMap<PackageId, Arc<PackageExports>>,
			       downloaded_graph: HashMap<&PackageId, &HashSet<Importer>>| {
				let mut tasks = downloaded_graph
					.into_iter()
					.map(|(id, importers)| {
						let subproject = self
							.clone()
							// importer does not matter here, as it is the same package being linked in different places
							.subproject(importers.iter().next().unwrap().clone());
						let structure_kind = graph.nodes[id].structure_kind.clone();
						let install_path = subproject
							.dependencies_dir()
							.join(graph.realm_of(subproject.importer(), id).packages_dir())
							.join(PACKAGES_CONTAINER_NAME)
							.join(DependencyGraphNode::container_dir(id, &structure_kind))
							.into();
						let project = self.clone();
						let id = id.clone();

						async move {
							let exports = id
								.source()
								.get_exports(
									id.pkg_ref(),
									&GetExportsOptions {
										project,
										path: install_path,
										version: id.version(),
										structure_kind: &structure_kind,
									},
								)
								.await?;

							Ok::<_, errors::DownloadAndLinkError>((id, exports))
						}
					})
					.collect::<JoinSet<_>>();

				while let Some(task) = tasks.join_next().await {
					let (id, exports) = task.unwrap()?;
					package_exports.insert(id, Arc::new(exports));
				}

				Ok::<_, errors::DownloadAndLinkError>(())
			};

		// step 2. get targets for non Wally packages (Wally packages require the scripts packages to be downloaded first)
		get_graph_exports(&mut package_exports, other_graph_to_download)
			.instrument(tracing::debug_span!("get targets (non-wally)"))
			.await?;

		self.link(graph, &package_exports, &Default::default())
			.instrument(tracing::debug_span!("link (non-wally)"))
			.await?;

		// step 3. get targets for Wally packages
		get_graph_exports(&mut package_exports, wally_graph_to_download)
			.instrument(tracing::debug_span!("get targets (wally)"))
			.await?;

		let mut tasks = package_exports
			.iter()
			.map(|(id, exports)| {
				// importer does not matter here, as it is the same package being linked in different places
				let subproject = self
					.clone()
					.subproject(graph_to_download[id].iter().next().unwrap().clone());
				let install_path = subproject
					.dependencies_dir()
					.join(graph.realm_of(subproject.importer(), id).packages_dir())
					.join(PACKAGES_CONTAINER_NAME)
					.join(DependencyGraphNode::container_dir(
						id,
						&graph.nodes[id].structure_kind,
					));

				let span = tracing::info_span!("extract types", package_id = id.to_string());

				let package_id = id.clone();
				let exports = exports.clone();

				async move {
					let Some(lib_file) = exports.lib_file.as_deref() else {
						return Ok((package_id, vec![]));
					};

					let types = if lib_file.as_str() == LINK_LIB_NO_FILE_FOUND {
						vec![]
					} else {
						let lib_file = lib_file.to_path(install_path);

						let contents = match fs::read_to_string(&lib_file).await {
							Ok(contents) => contents,
							Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
								return Err(errors::DownloadAndLinkErrorKind::LibFileNotFound(
									lib_file,
								)
								.into());
							}
							Err(e) => return Err(e.into()),
						};

						let types = spawn_blocking(move || get_file_types(&contents))
							.await
							.unwrap();

						tracing::debug!("contains {} exported types", types.len());

						types
					};

					Ok::<_, errors::DownloadAndLinkError>((package_id, types))
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
		self.link(graph, &package_exports, &package_types)
			.instrument(tracing::debug_span!("link (all)"))
			.await?;

		if matches!(install_dependencies_mode, InstallDependenciesMode::Prod) || !force {
			self.remove_unused(graph).await?;
		}

		Ok(package_exports)
	}
}

/// Errors that can occur when downloading and linking dependencies
pub mod errors {
	use std::path::PathBuf;

	use thiserror::Error;

	use crate::source::ids::PackageId;

	/// An error that can occur when downloading and linking dependencies
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadAndLinkError))]
	#[non_exhaustive]
	pub enum DownloadAndLinkErrorKind {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// An error occurred while downloading the graph
		#[error("error downloading graph")]
		DownloadGraph(#[from] crate::download::errors::DownloadGraphError),

		/// An error occurred while linking dependencies
		#[error("error linking dependencies")]
		Linking(#[from] crate::linking::errors::LinkingError),

		/// IO error
		#[error("io error")]
		Io(#[from] std::io::Error),

		/// Error getting package exports
		#[error("error getting package exports")]
		GetExports(#[from] crate::source::errors::GetExportsError),

		/// Removing unused dependencies failed
		#[error("error removing unused dependencies")]
		RemoveUnused(#[from] crate::linking::incremental::errors::RemoveUnusedError),

		/// Patching a package failed
		#[cfg(feature = "patches")]
		#[error("error applying patch")]
		Patch(#[from] crate::patches::errors::ApplyPatchError),

		/// The library file was not found
		#[error("library file at `{0}` not found")]
		LibFileNotFound(PathBuf),

		/// The checksum of the package is mismatched
		#[error("invalid checksum for `{0}`")]
		BadChecksum(PackageId),
	}
}
