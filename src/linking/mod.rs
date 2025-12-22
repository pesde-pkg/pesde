use crate::{
	PACKAGES_CONTAINER_NAME, Project,
	graph::{DependencyGraphNodeWithTarget, DependencyGraphWithTarget},
	manifest::{Alias, Manifest},
	source::{
		fs::{cas_path, store_in_cas},
		ids::PackageId,
		traits::PackageRef as _,
	},
};
use fs_err::tokio as fs;
use std::{
	collections::HashMap,
	path::{Path, PathBuf},
};
use tokio::task::JoinSet;

/// Generates linking modules for a project
pub mod generator;
/// Incremental installs
pub mod incremental;

async fn create_and_canonicalize<P: AsRef<Path>>(path: P) -> std::io::Result<PathBuf> {
	let p = path.as_ref();
	fs::create_dir_all(p).await?;
	fs::canonicalize(p).await
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
		graph: &DependencyGraphWithTarget,
		manifest: &Manifest,
		package_types: &HashMap<PackageId, Vec<String>>,
		is_complete: bool,
	) -> Result<(), errors::LinkingError> {
		let mut tasks = JoinSet::<Result<_, errors::LinkingError>>::new();
		let mut link_files = |base_folder: &Path,
		                      container_folder: &Path,
		                      root_container_folder: &Path,
		                      relative_container_folder: &Path,
		                      node: &DependencyGraphNodeWithTarget,
		                      package_id: &PackageId,
		                      alias: &Alias|
		 -> Result<(), errors::LinkingError> {
			static NO_TYPES: Vec<String> = Vec::new();

			if let Some(lib_file) = node.target.lib_path() {
				let destination = base_folder.join(format!("{alias}.luau"));

				let lib_module = generator::generate_lib_linking_module(
					&generator::get_lib_require_path(
						node.target.kind(),
						base_folder,
						lib_file,
						container_folder,
						node.node.resolved.pkg_ref.structure_kind(),
						root_container_folder,
						relative_container_folder,
						manifest,
					)?,
					package_types.get(package_id).unwrap_or(&NO_TYPES),
				);
				let cas_dir = self.cas_dir().to_path_buf();

				tasks.spawn(async move {
					write_cas(destination, &cas_dir, &lib_module)
						.await
						.map_err(Into::into)
				});
			}

			if let Some(bin_file) = node.target.bin_path() {
				let destination = base_folder.join(format!("{alias}.bin.luau"));

				let bin_module = generator::generate_bin_linking_module(
					container_folder,
					&generator::get_bin_require_path(base_folder, bin_file, container_folder),
				);
				let cas_dir = self.cas_dir().to_path_buf();

				tasks.spawn(async move {
					write_cas(destination, &cas_dir, &bin_module)
						.await
						.map_err(Into::into)
				});
			}

			Ok(())
		};

		let mut node_tasks = graph
			.iter()
			.map(|(id, node)| {
				let base_folder = self
					.package_dir()
					.join(manifest.target.kind().packages_folder(id.v_id().target()));

				let id = id.clone();
				let node = node.clone();

				async move {
					Ok::<_, errors::LinkingError>((
						id,
						node,
						create_and_canonicalize(base_folder).await?,
					))
				}
			})
			.collect::<JoinSet<_>>();

		let mut dependency_tasks = JoinSet::<Result<_, errors::LinkingError>>::new();

		loop {
			tokio::select! {
				Some(res) = node_tasks.join_next() => {
					let (package_id, node, base_folder) = res.unwrap()?;
					let (node_container_folder, node_packages_folder) = {
						let packages_container_folder = base_folder.join(PACKAGES_CONTAINER_NAME);

						let container_folder =
							packages_container_folder.join(node.node.container_folder(&package_id));

						if let Some((alias, _, _)) = &node.node.direct {
							link_files(
								&base_folder,
								&container_folder,
								&base_folder,
								container_folder.strip_prefix(&base_folder).unwrap(),
								&node,
								&package_id,
								alias,
							)?;
						}

						(container_folder, base_folder)
					};

					for (dep_alias, dep_id) in &node.node.resolved_dependencies {
						let dep_id = dep_id.clone();
						let dep_alias = dep_alias.clone();
						let dep_node = graph.get(&dep_id).cloned();
						let node = node.clone();
						let package_id = package_id.clone();
						let node_container_folder = node_container_folder.clone();
						let node_packages_folder = node_packages_folder.clone();
						let package_dir = self.package_dir().to_path_buf();

						dependency_tasks.spawn(async move {
							let Some(dep_node) = dep_node else {
								return if is_complete {
									Err(errors::LinkingError::DependencyNotFound(
										dep_id.to_string(),
										package_id.to_string(),
									))
								} else {
									Ok(None)
								};
							};

							let base_folder = package_dir.join(
								package_id
									.v_id()
									.target()
									.packages_folder(dep_id.v_id().target()),
							);
							let linker_folder = node_container_folder.join(node.node.dependencies_dir(
								package_id.v_id(),
								dep_id.v_id().target(),
							));

							Ok(Some((
								dep_node.clone(),
								dep_id,
								dep_alias,
								create_and_canonicalize(base_folder).await?,
								create_and_canonicalize(linker_folder).await?,
								node_packages_folder,
							)))
						});
					}
				},
				Some(res) = dependency_tasks.join_next() => {
					let Some((
						dependency_node,
						dependency_id,
						dependency_alias,
						base_folder,
						linker_folder,
						node_packages_folder,
					)) = res.unwrap()?
					else {
						continue;
					};

					let packages_container_folder = base_folder.join(PACKAGES_CONTAINER_NAME);

					let container_folder = packages_container_folder
						.join(dependency_node.node.container_folder(&dependency_id));

					link_files(
						&linker_folder,
						&container_folder,
						&node_packages_folder,
						container_folder.strip_prefix(&base_folder).unwrap(),
						&dependency_node,
						&dependency_id,
						&dependency_alias,
					)?;
				},
				else => break,
			}
		}

		while let Some(task) = tasks.join_next().await {
			task.unwrap()?;
		}

		Ok(())
	}
}

/// Errors that can occur while linking dependencies
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur while linking dependencies
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum LinkingError {
		/// An error occurred while interacting with the filesystem
		#[error("error interacting with filesystem")]
		Io(#[from] std::io::Error),

		/// A dependency was not found
		#[error("dependency `{0}` of `{1}` not found")]
		DependencyNotFound(String, String),

		/// An error occurred while getting the require path for a library
		#[error("error getting require path for library")]
		GetLibRequirePath(#[from] super::generator::errors::GetLibRequirePath),
	}
}
