use crate::{
	PACKAGES_CONTAINER_NAME, Project,
	graph::{DependencyGraph, DependencyGraphNode},
	linking::generator::LinkDirs,
	manifest::{Manifest, target::Target},
	source::{
		fs::{cas_path, store_in_cas},
		ids::PackageId,
		traits::PackageRef as _,
	},
};
use fs_err::tokio as fs;
use futures::StreamExt as _;
use relative_path::RelativePath;
use std::{
	collections::HashMap,
	path::{Path, PathBuf},
	sync::Arc,
};
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
		let mut importer_manifests = HashMap::<Arc<RelativePath>, Arc<Manifest>>::new();
		let members = self.workspace_members().await?;
		tokio::pin!(members);
		while let Some((importer, manifest)) = members.next().await.transpose()? {
			importer_manifests.insert(importer.into(), manifest.into());
		}

		let mut node_tasks = graph
			.importers
			.iter()
			.flat_map(|(importer, dependencies)| {
				dependencies
					.iter()
					.filter(|(_, (id, _, _))| graph.nodes.contains_key(id))
					.map(|(alias, (id, _, _))| {
						let importer = importer.clone();
						let dependencies_dir = importer
							.to_path(self.private_dir())
							.join("dependencies")
							.join(id.v_id().target().packages_dir());

						let container_dir = PathBuf::from(PACKAGES_CONTAINER_NAME)
							.join(DependencyGraphNode::container_dir(id));

						(
							importer,
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
						dependencies
							.values()
							.filter_map(|(id, _, _)| graph.nodes.get(id).map(|node| (id, node)))
							.flat_map(|(id, node)| {
								node.dependencies
									.iter()
									.map(|(dep_alias, dep_id)| (id.clone(), dep_alias, dep_id))
							})
							.map(|(dependant_id, dep_alias, dep_id)| {
								let importer = importer.clone();
								let dependencies_dir =
									importer.to_path(self.private_dir()).join("dependencies");

								let container_dir = PathBuf::from(PACKAGES_CONTAINER_NAME)
									.join(DependencyGraphNode::container_dir(dep_id));

								(
									importer,
									dep_alias.clone(),
									dep_id.clone(),
									LinkDirs {
										base: dependencies_dir
											.join(dependant_id.v_id().target().packages_dir())
											.join(PACKAGES_CONTAINER_NAME)
											.join(DependencyGraphNode::container_dir(&dependant_id))
											.join(DependencyGraphNode::dependencies_dir(
												&dependant_id,
											)),
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
			.filter_map(|(importer, alias, id, dirs)| {
				let project = self.clone();
				let target = package_targets.get(&id).cloned()?;
				let manifest = importer_manifests[&importer].clone();
				let types = package_types.get(&id).cloned();

				Some(async move {
					static NO_TYPES: [String; 0] = [];

					let mut tasks = JoinSet::<Result<_, errors::LinkingError>>::new();

					if target.lib_path().is_some() || target.bin_path().is_some() {
						fs::create_dir_all(&dirs.base).await?;
					}

					if let Some(lib_file) = target.lib_path() {
						let destination =
							dirs.base.join(alias.as_str()).with_added_extension("luau");

						let lib_module = generator::generate_lib_linking_module(
							&generator::get_lib_require_path(
								target.kind(),
								lib_file,
								&dirs,
								id.pkg_ref().structure_kind(),
								&manifest,
							)?,
							types.as_deref().unwrap_or(&NO_TYPES),
						);
						let cas_dir = project.cas_dir().to_path_buf();

						tasks.spawn(async move {
							write_cas(destination, &cas_dir, &lib_module)
								.await
								.map_err(Into::into)
						});
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
						let cas_dir = project.cas_dir().to_path_buf();

						tasks.spawn(async move {
							write_cas(destination, &cas_dir, &bin_module)
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

	/// Errors that can occur while linking dependencies
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum LinkingError {
		/// An error occurred while interacting with the filesystem
		#[error("error interacting with filesystem")]
		Io(#[from] std::io::Error),

		/// An error occurred while getting the require path for a library
		#[error("error getting require path for library")]
		GetLibRequirePath(#[from] super::generator::errors::GetLibRequirePath),

		/// An error occurred while getting the workspace members
		#[error("error getting workspace members")]
		WorkspaceMembers(#[from] crate::errors::WorkspaceMembersError),
	}
}
