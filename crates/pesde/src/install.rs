//! Full project setup process - downloading packages and linking them
use crate::Error;
use crate::Project;
use crate::Resolution;
use crate::Result;

use crate::link::LinkVfs;
use crate::link::PackageLinkContext;
use crate::link::PackageStructure;
use crate::registry::manifest::DependencyType;
use crate::reporters::DownloadProgressReporter;
use crate::reporters::DownloadsReporter;
use crate::reporters::PatchesReporter;
use crate::source::PackageId;
use crate::source::PackageSource;

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::task::JoinSet;
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

/// Options for installing a project.
#[derive(Debug)]
pub struct InstallOptions<Reporter = ()> {
	/// The downloads reporter.
	pub reporter: Option<Arc<Reporter>>,
	/// Which dependencies to install.
	pub install_dependencies_mode: InstallDependenciesMode,
	/// Whether to re-install all dependencies even if they are already installed
	pub force: bool,
}

impl<Reporter> Default for InstallOptions<Reporter>
where
	Reporter: DownloadsReporter + PatchesReporter + Send + Sync + 'static,
{
	fn default() -> Self {
		Self {
			reporter: None,
			install_dependencies_mode: InstallDependenciesMode::All,
			force: false,
		}
	}
}

impl<Reporter> InstallOptions<Reporter>
where
	Reporter: DownloadsReporter + PatchesReporter + Send + Sync + 'static,
{
	/// Creates a new download options.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Sets the downloads reporter.
	#[must_use]
	pub fn reporter(mut self, reporter: impl Into<Arc<Reporter>>) -> Self {
		self.reporter.replace(reporter.into());
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

	/// Sets whether to re-install all dependencies even if they are already installed
	#[must_use]
	pub fn force(mut self, force: bool) -> Self {
		self.force = force;
		self
	}
}

impl Clone for InstallOptions {
	fn clone(&self) -> Self {
		Self {
			reporter: self.reporter.clone(),
			install_dependencies_mode: self.install_dependencies_mode,
			force: self.force,
		}
	}
}

impl Project {
	/// Installs the dependencies of the project.
	#[instrument(
		skip_all,
		fields(install_dependencies = debug(options.install_dependencies_mode)),
		level = "debug"
	)]
	pub async fn install<Reporter>(
		&self,
		resolution: &Resolution,
		options: InstallOptions<Reporter>,
	) -> Result<()>
	where
		Reporter: DownloadsReporter + PatchesReporter + 'static,
	{
		let InstallOptions {
			reporter,
			install_dependencies_mode,
			force,
		} = options;

		if force {
			let mut tasks = resolution
				.importers
				.keys()
				.map(|importer| {
					let member = self.member(importer.clone());
					let link_vfs = LinkVfs::new(member);

					async move { link_vfs.clean_dependencies_dir().await }
				})
				.collect::<JoinSet<_>>();

			while let Some(task) = tasks.join_next().await {
				task.unwrap()?;
			}
		}

		struct InstalledPackage {
			used_by: Vec<LinkVfs>,
			source: Option<Arc<dyn PackageSource>>,
			structure: PackageStructure,
		}

		// step 1. install packages into their subprojects'
		let mut importer_deps = HashMap::<PackageId, Vec<LinkVfs>>::new();
		let mut visited = HashSet::new();
		let mut queue = vec![];
		for (importer, graph_importer) in &resolution.importers {
			queue.extend(
				graph_importer
					.dependencies
					.values()
					.filter(|(_, _, ty)| install_dependencies_mode.fits(*ty))
					.map(|(id, _, _)| id.clone()),
			);

			while let Some(pkg_id) = queue.pop() {
				if let Some(node) = resolution.graph.get(&pkg_id)
					&& visited.insert(pkg_id)
				{
					for (dep_id, dep_ty) in node.dependencies.values() {
						// don't need to download dev dependencies of dependencies
						if *dep_ty == DependencyType::Dev {
							continue;
						}
						queue.push(dep_id.clone());
					}
				}
			}

			for id in visited.drain() {
				let member = self.member(importer.clone());
				let link_vfs = LinkVfs::new(member);

				importer_deps.entry(id).or_default().push(link_vfs);
			}
		}

		let mut installed_packages = HashMap::<PackageId, InstalledPackage>::new();
		let mut tasks = JoinSet::new();
		for id in importer_deps.keys() {
			let id = id.clone();

			if let Some(structure) = resolution.graph[&id].structure.clone() {
				installed_packages.insert(
					id,
					InstalledPackage {
						used_by: Vec::new(),
						source: None,
						structure,
					},
				);
			} else {
				let project = self.clone();

				tasks.spawn(async move {
					let source = project
						.source_provider()
						.get(&id.source_id())
						.await
						.map_err(Error::source)?;

					let inspect_result = source.inspect(&id).await.map_err(Error::source)?;
					Ok::<_, Error>((
						id,
						InstalledPackage {
							used_by: Vec::new(),
							source: Some(source),
							structure: inspect_result.structure,
						},
					))
				});
			}
		}

		while let Some(task) = tasks.join_next().await {
			let (id, installed_package) = task.unwrap()?;
			installed_packages.insert(id, installed_package);
		}

		let graph_to_download = if force {
			importer_deps
		} else {
			let mut tasks = importer_deps
				.into_iter()
				.flat_map(|(id, users)| {
					users
						.into_iter()
						.map(move |link_vfs| (link_vfs, id.clone()))
				})
				.map(|(link_vfs, id)| {
					async move {
						// if id.pkg_ref().is_local() {
						// 	return (importer, id, false);
						// }

						let result = link_vfs.package_dir_exists(&id).await;
						(link_vfs, id, result)
					}
				})
				.collect::<JoinSet<_>>();

			let mut deps_to_download = HashMap::<PackageId, Vec<LinkVfs>>::new();
			while let Some(task) = tasks.join_next().await {
				let (link_vfs, id, installed) = task.unwrap();
				if installed? {
					installed_packages
						.get_mut(&id)
						.unwrap()
						.used_by
						.push(link_vfs);
					continue;
				}

				deps_to_download.entry(id).or_default().push(link_vfs);
			}

			deps_to_download
		};

		let mut tasks = graph_to_download
			.into_iter()
			.map(|(package_id, needing_install)| {
				let span = tracing::info_span!("install", package_id = package_id.to_string());

				let reporter = reporter.clone();

				let mut installed = installed_packages.remove(&package_id).unwrap();

				let project = self.clone();

				async move {
					let mut consumers = needing_install.iter();
					// doesn't matter which one, we use it to atomically probe whether we need to download the package
					let first_link_vfs = consumers.next().unwrap();

					let source = match installed.source.take() {
						Some(source) => source,
						None => project
							.source_provider()
							.get(&package_id.source_id())
							.await
							.map_err(Error::source)?,
					};

					let storer = first_link_vfs
						.install_package(&package_id, &installed.structure, true)
						.await?;

					if let Some(mut storer) = storer {
						tracing::debug!("downloading");

						let progress_reporter = reporter.map_or_else(
							|| Arc::new(()) as Arc<dyn DownloadProgressReporter + 'static>,
							|reporter| Arc::new(reporter.report_download(package_id.to_string())),
						);
						progress_reporter.report_start();

						source
							.download(&package_id, progress_reporter, &mut *storer)
							.await
							.map_err(Error::source)?;

						tracing::debug!("downloaded");

						storer.commit().await.map_err(Error::source)?;
					}

					for link_vfs in consumers {
						link_vfs
							.install_package(&package_id, &installed.structure, false)
							.await?;
					}

					installed.used_by.extend(needing_install);

					Ok((package_id, installed))
				}
				.instrument(span)
			})
			.collect::<JoinSet<Result<_>>>();

		while let Some(task) = tasks.join_next().await {
			let (id, installed) = task.unwrap()?;
			installed_packages.insert(id, installed);
		}

		// TODO: apply patches
		// TODO: reconsider how packagestructure works/where it originates from.

		// step 2. link packages into their subprojects' dependencies.
		// for now, do so without context. this allows the link hook to execute
		// (context is meant to improve the final result, linking should be possible without it)
		for (id, installed) in &installed_packages {
			let link_context = PackageLinkContext {
				package_id: id,
				structure: &installed.structure,
				context: "",
			};

			let dependencies =
				resolution.graph[id]
					.dependencies
					.iter()
					.map(|(alias, (dep_id, _))| {
						(
							alias,
							PackageLinkContext {
								package_id: dep_id,
								structure: &installed_packages[dep_id].structure,
								context: "",
							},
						)
					});

			for link_vfs in &installed.used_by {
				link_vfs
					.link_package_depenencies(link_context, dependencies.clone())
					.await?;
			}
		}

		for (importer, importer_data) in &resolution.importers {
			let member = self.member(importer.clone());
			let link_vfs = LinkVfs::new(member);

			link_vfs
				.link_subproject(importer_data.dependencies.iter().filter_map(
					|(alias, (dep_id, _, _))| {
						installed_packages
							.get(dep_id)
							.map(|dep| (alias, dep_id, &dep.structure))
					},
				))
				.await?;

			if matches!(install_dependencies_mode, InstallDependenciesMode::Prod) || !force {
				link_vfs
					.prune_unused_packages(installed_packages.keys())
					.await?;
			}
		}

		Ok(())
	}
}
