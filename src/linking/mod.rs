use crate::PACKAGES_CONTAINER_NAME;
use crate::Project;
use crate::linking::generator::LinkDirs;
use crate::resolver::DependencyGraph;
use crate::resolver::DependencyGraphNode;
use crate::source::PackageRefs;
use crate::source::RealmExt as _;
use crate::source::StructureKind;
use crate::source::fs::cas_path;
use crate::source::fs::store_in_cas;
use crate::source::ids::PackageId;
use crate::source::traits::PackageExports;
use crate::source::traits::PackageRef as _;
use crate::util::ToEscaped as _;
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

impl DependencyGraphNode {
	/// Returns the directory to store the contents of the package in, e.g. foo+1.0.0/1.0.0
	#[must_use]
	pub fn container_dir(package_id: &PackageId) -> PathBuf {
		PathBuf::from(package_id.to_string().escaped())
			.join(package_id.version().to_string().escaped())
	}
}

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
		package_exports: &HashMap<PackageId, Arc<PackageExports>>,
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
						let realm = graph.realm_of(importer, id);
						let dependencies_dir =
							subproject.dependencies_dir().join(realm.packages_dir());

						let container_dir = PathBuf::from(PACKAGES_CONTAINER_NAME)
							.join(DependencyGraphNode::container_dir(id));

						(
							subproject,
							alias.clone(),
							id.clone(),
							realm,
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
								node.dependencies.iter().map(|(dep_alias, (dep_id, _, _))| {
									(id.clone(), dep_alias, dep_id)
								})
							})
							.map(|(dependant_id, dep_alias, dep_id)| {
								let subproject = self.clone().subproject(importer.clone());
								let dependencies_dir = subproject.dependencies_dir();
								let dep_realm = graph.realm_of(importer, dep_id);
								let dependant_realm = graph.realm_of(importer, &dependant_id);

								let container_dir = PathBuf::from(PACKAGES_CONTAINER_NAME)
									.join(DependencyGraphNode::container_dir(dep_id));

								(
									subproject,
									dep_alias.clone(),
									dep_id.clone(),
									dep_realm,
									LinkDirs {
										#[expect(deprecated)]
										base: dependencies_dir
											.join(dependant_realm.packages_dir())
											.join(PACKAGES_CONTAINER_NAME)
											.join(DependencyGraphNode::container_dir(&dependant_id))
											.join(match dependant_id.pkg_ref().structure_kind() {
												StructureKind::Wally => "..",
												StructureKind::PesdeV1(_) => match dep_id.pkg_ref()
												{
													PackageRefs::Pesde(pkg_ref) => {
														pkg_ref.target.packages_dir()
													}
													PackageRefs::Wally(_) => panic!(
														"unable to link wally package to pesde_v1 package, do not know how to link"
													),
													PackageRefs::Git(pkg_ref) => {
														match pkg_ref.structure_kind {
															StructureKind::Wally => "..",
															StructureKind::PesdeV1(target) => {
																target.packages_dir()
															}
															StructureKind::PesdeV2 => panic!(
																"pesde_v1 depends on pesde_v2, do not know how to link"
															),
														}
													}
													PackageRefs::Path(_) => unreachable!(),
												},
												StructureKind::PesdeV2 => {
													// TODO: use luaurc aliases
													dependant_realm.packages_dir()
												}
											}),
										destination: dependencies_dir
											.join(dep_realm.packages_dir())
											.join(&container_dir),
										container: container_dir,
										root_container: dependencies_dir
											.join(dependant_realm.packages_dir()),
									},
								)
							}),
					)
			})
			.filter_map(|(subproject, alias, id, realm, dirs)| {
				let exports = package_exports.get(&id).cloned()?;
				let types = package_types.get(&id).cloned();

				Some(async move {
					static NO_TYPES: [String; 0] = [];

					let mut tasks = JoinSet::<Result<_, errors::LinkingError>>::new();

					if exports.lib.is_some() || exports.bin.is_some() {
						fs::create_dir_all(&dirs.base).await?;
					}

					if let Some(bin_file) = exports.bin.as_deref() {
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

					if let Some(lib_file) = exports.lib.clone() {
						let destination =
							dirs.base.join(alias.as_str()).with_added_extension("luau");

						let cas_dir = subproject.project().cas_dir().to_path_buf();

						tasks.spawn(async move {
							let lib_module = generator::generate_lib_linking_module(
								&generator::get_lib_require_path(
									realm,
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
