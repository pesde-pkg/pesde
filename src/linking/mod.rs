use crate::PACKAGES_CONTAINER_NAME;
use crate::Project;
use crate::graph::DependencyGraph;
use crate::graph::DependencyGraphNode;
use crate::linking::generator::LinkDirs;
use crate::manifest::target::Target;
use crate::source::StructureKind;
use crate::source::fs::cas_path;
use crate::source::fs::store_in_cas;
use crate::source::ids::PackageId;
use crate::source::traits::PackageRef as _;
use fs_err::tokio as fs;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinSet;

/// Generates linking modules for a project
pub mod generator;
/// Incremental installs
pub mod incremental;

async fn write_cas(destination: PathBuf, cas_dir: &Path, contents: &str) -> std::io::Result<()> {
	let hash = store_in_cas(cas_dir, contents.as_bytes()).await?;

	match fs::remove_file(&destination).await {
		Ok(_) => {}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
		// TODO: investigate why this happens and whether we can avoid it without ignoring all PermissionDenied errors
		#[cfg(windows)]
		Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {}
		Err(e) => return Err(e),
	}

	fs::hard_link(cas_path(&hash, cas_dir), destination).await
}

impl Project {
	pub(crate) async fn link(
		&self,
		graph: &DependencyGraph,
		package_targets: &HashMap<PackageId, Arc<Target>>,
		package_types: &HashMap<PackageId, Arc<[String]>>,
	) -> Result<(), errors::LinkingError> {
		let mut node_tasks = graph
			.importers
			.iter()
			.flat_map(|(importer, data)| {
				data.dependencies
					.iter()
					.filter(|(_, (id, _, _))| graph.nodes.contains_key(id))
					.map(|(alias, (id, _, _))| {
						let subproject = self.clone().subproject(importer.clone());
						let dependencies_dir = subproject
							.dependencies_dir()
							.join(id.v_id().target().packages_dir());

						let container_dir = PathBuf::from(PACKAGES_CONTAINER_NAME)
							.join(DependencyGraphNode::container_dir(id));

						(
							subproject,
							alias.clone(),
							id.clone(),
							LinkDirs {
								base: dependencies_dir.clone(),
								destination: dependencies_dir.join(&container_dir),
								container: container_dir,
								root_container: dependencies_dir,
							},
						)
					})
					.chain(
						data.dependencies
							.values()
							.filter_map(|(id, _, _)| graph.nodes.get(id).map(|node| (id, node)))
							.flat_map(|(id, node)| {
								node.dependencies
									.iter()
									.map(|(dep_alias, dep_id)| (id.clone(), dep_alias, dep_id))
							})
							.map(|(dependant_id, dep_alias, dep_id)| {
								let subproject = self.clone().subproject(importer.clone());
								let dependencies_dir = subproject.dependencies_dir();

								let container_dir = PathBuf::from(PACKAGES_CONTAINER_NAME)
									.join(DependencyGraphNode::container_dir(dep_id));

								(
									subproject,
									dep_alias.clone(),
									dep_id.clone(),
									LinkDirs {
										base: dependencies_dir
											.join(dependant_id.v_id().target().packages_dir())
											.join(PACKAGES_CONTAINER_NAME)
											.join(DependencyGraphNode::container_dir(&dependant_id))
											.join(match dependant_id.pkg_ref().structure_kind() {
												StructureKind::Wally => "..",
												StructureKind::PesdeV1 => {
													dep_id.v_id().target().packages_dir()
												}
											}),
										destination: dependencies_dir
											.join(dep_id.v_id().target().packages_dir())
											.join(&container_dir),
										container: container_dir,
										root_container: dependencies_dir
											.join(dependant_id.v_id().target().packages_dir()),
									},
								)
							}),
					)
			})
			.filter_map(|(subproject, alias, id, dirs)| {
				let target = package_targets.get(&id).cloned()?;
				let types = package_types.get(&id).cloned();

				Some(async move {
					static NO_TYPES: [String; 0] = [];

					let mut tasks = JoinSet::<Result<_, errors::LinkingError>>::new();

					if target.lib_path().is_some() || target.bin_path().is_some() {
						fs::create_dir_all(&dirs.base).await?;
					}

					if let Some(bin_file) = target.bin_path() {
						let destination = dirs
							.base
							.join(alias.as_str())
							.with_added_extension("bin.luau");

						let bin_module = generator::generate_bin_linking_module(
							&dirs.container,
							&generator::get_bin_require_path(&dirs.base, bin_file, &dirs.container),
						);
						let cas_dir = subproject.project().cas_dir().to_path_buf();

						tasks.spawn(async move {
							write_cas(destination, &cas_dir, &bin_module)
								.await
								.map_err(Into::into)
						});
					}

					if let Some(lib_file) = target.lib_path() {
						let destination =
							dirs.base.join(alias.as_str()).with_added_extension("luau");

						let cas_dir = subproject.project().cas_dir().to_path_buf();
						let lib_file = lib_file.to_relative_path_buf();
						let target_kind = target.kind();

						tasks.spawn(async move {
							let lib_module = generator::generate_lib_linking_module(
								&generator::get_lib_require_path(
									target_kind,
									&lib_file,
									&dirs,
									id.pkg_ref().structure_kind(),
									&*subproject.deser_manifest().await?,
								)
								.map_err(|e| {
									errors::LinkingErrorKind::GetLibRequirePath(
										id.clone(),
										subproject.importer().clone(),
										e,
									)
								})?,
								types.as_deref().unwrap_or(&NO_TYPES),
							);

							write_cas(destination, &cas_dir, &lib_module)
								.await
								.map_err(Into::into)
						});
					}

					while let Some(task) = tasks.join_next().await {
						task.unwrap()?;
					}

					Ok::<_, errors::LinkingError>(())
				})
			})
			.collect::<JoinSet<_>>();

		while let Some(task) = node_tasks.join_next().await {
			task.unwrap()?;
		}

		Ok(())
	}
}

/// Errors that can occur while linking dependencies
pub mod errors {
	use thiserror::Error;

	use crate::Importer;
	use crate::source::ids::PackageId;

	/// Errors that can occur while linking dependencies
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = LinkingError))]
	#[non_exhaustive]
	pub enum LinkingErrorKind {
		/// An error occurred while interacting with the filesystem
		#[error("error interacting with filesystem")]
		Io(#[from] std::io::Error),

		/// An error occurred while getting the require path for a library
		#[error("error getting require path for `{0}` in importer `{1}`")]
		GetLibRequirePath(
			PackageId,
			Importer,
			#[source] super::generator::errors::GetLibRequirePath,
		),

		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),
	}
}
