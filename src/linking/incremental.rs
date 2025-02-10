use crate::{
	all_packages_dirs, graph::DependencyGraphWithTarget, manifest::Alias, source::ids::PackageId,
	util::remove_empty_dir, Project, PACKAGES_CONTAINER_NAME, SCRIPTS_LINK_FOLDER,
};
use fs_err::tokio as fs;
use futures::FutureExt;
use std::{
	collections::HashSet,
	path::{Component, Path, PathBuf},
	sync::Arc,
};
use tokio::task::JoinSet;

fn index_entry(
	entry: fs::DirEntry,
	packages_index_dir: &Path,
	tasks: &mut JoinSet<Result<(), errors::RemoveUnusedError>>,
	used_paths: &Arc<HashSet<PathBuf>>,
	#[cfg(feature = "patches")] patched_packages: &Arc<HashSet<PathBuf>>,
) {
	fn get_package_name_from_container(container: &Path) -> (bool, String) {
		let Component::Normal(first_component) = container.components().next().unwrap() else {
			panic!("invalid container path: `{}`", container.display());
		};

		let first_component = first_component.to_string_lossy();
		let Some((name, _)) = first_component.split_once('@') else {
			return (
				false,
				first_component.split_once('+').unwrap().1.to_string(),
			);
		};

		(true, name.split_once('_').unwrap().1.to_string())
	}

	let path = entry.path();
	let path_relative = path.strip_prefix(packages_index_dir).unwrap().to_path_buf();

	let (is_wally, package_name) = get_package_name_from_container(&path_relative);

	let used_paths = used_paths.clone();
	let patched_packages = patched_packages.clone();
	tasks.spawn(async move {
		if is_wally {
			#[cfg(not(feature = "wally-compat"))]
			{
				tracing::error!(
					"found Wally package in index despite feature being disabled at `{}`",
					path.display()
				);
			}
			#[cfg(feature = "wally-compat")]
			{
				if !used_paths.contains(&path_relative) {
					fs::remove_dir_all(path).await?;
				} else {
					#[cfg(feature = "patches")]
					if !patched_packages.contains(&path_relative) {
						crate::patches::remove_patch(path.join(package_name)).await?;
					}
				}

				return Ok(());
			}
		}

		let mut tasks = JoinSet::<Result<_, errors::RemoveUnusedError>>::new();

		let mut entries = fs::read_dir(&path).await?;
		while let Some(entry) = entries.next_entry().await? {
			let version = entry.file_name();
			let path_relative = path_relative.join(&version);

			if used_paths.contains(&path_relative) {
				#[cfg(feature = "patches")]
				if !patched_packages.contains(&path_relative) {
					let path = entry.path().join(&package_name);
					tasks.spawn(async {
						crate::patches::remove_patch(path).await.map_err(Into::into)
					});
				}
				continue;
			}

			let path = entry.path();
			tasks.spawn(async { fs::remove_dir_all(path).await.map_err(Into::into) });
		}

		while let Some(task) = tasks.join_next().await {
			task.unwrap()?;
		}

		remove_empty_dir(&path).await.map_err(Into::into)
	});
}

fn packages_entry(
	entry: fs::DirEntry,
	tasks: &mut JoinSet<Result<(), errors::RemoveUnusedError>>,
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

fn scripts_entry(
	entry: fs::DirEntry,
	tasks: &mut JoinSet<Result<(), errors::RemoveUnusedError>>,
	expected_aliases: &Arc<HashSet<Alias>>,
) {
	let expected_aliases = expected_aliases.clone();
	tasks.spawn(async move {
		if !entry.file_type().await?.is_dir() {
			return Ok(());
		}

		let path = entry.path();
		let name = path
			.file_name()
			.unwrap()
			.to_str()
			.expect("non UTF-8 file name in scripts folder");
		let name = match name.parse::<Alias>() {
			Ok(name) => name,
			Err(e) => {
				tracing::error!("invalid alias in scripts folder: {e}");
				return Ok(());
			}
		};

		if !expected_aliases.contains(&name) {
			fs::remove_dir_all(&path).await?;
		}

		Ok(())
	});
}

impl Project {
	/// Removes unused packages from the project
	pub async fn remove_unused(
		&self,
		graph: &DependencyGraphWithTarget,
	) -> Result<(), errors::RemoveUnusedError> {
		let manifest = self.deser_manifest().await?;
		let used_paths = graph
			.iter()
			.map(|(id, node)| {
				node.node
					.container_folder(id)
					.parent()
					.unwrap()
					.to_path_buf()
			})
			.collect::<HashSet<_>>();
		let used_paths = Arc::new(used_paths);
		#[cfg(feature = "patches")]
		let patched_packages = manifest
			.patches
			.iter()
			.flat_map(|(name, versions)| {
				versions
					.iter()
					.map(|(v_id, _)| PackageId::new(name.clone(), v_id.clone()))
			})
			.filter_map(|id| graph.get(&id).map(|node| (id, node)))
			.map(|(id, node)| {
				node.node
					.container_folder(&id)
					.parent()
					.unwrap()
					.to_path_buf()
			})
			.collect::<HashSet<_>>();
		#[cfg(feature = "patches")]
		let patched_packages = Arc::new(patched_packages);

		let mut tasks = all_packages_dirs()
			.into_iter()
			.map(|folder| {
				let packages_dir = self.package_dir().join(&folder);
				let packages_index_dir = packages_dir.join(PACKAGES_CONTAINER_NAME);
				let used_paths = used_paths.clone();
				#[cfg(feature = "patches")]
				let patched_packages = patched_packages.clone();

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
						Err(e) => return Err(e.into()),
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
									#[cfg(feature = "patches")]
									&patched_packages,
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

					Ok::<_, errors::RemoveUnusedError>(())
				}
			})
			.collect::<JoinSet<_>>();

		let scripts_dir = self.package_dir().join(SCRIPTS_LINK_FOLDER);
		match fs::read_dir(&scripts_dir).await {
			Ok(mut entries) => {
				let expected_aliases = graph
					.iter()
					.filter_map(|(_, node)| {
						node.node
							.direct
							.as_ref()
							.map(|(alias, _, _)| alias.clone())
							.filter(|_| node.target.scripts().is_some_and(|s| !s.is_empty()))
					})
					.collect::<HashSet<_>>();
				let expected_aliases = Arc::new(expected_aliases);

				while let Some(entry) = entries.next_entry().await? {
					scripts_entry(entry, &mut tasks, &expected_aliases);
				}
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(e.into()),
		}

		while let Some(task) = tasks.join_next().await {
			task.unwrap()?;
		}

		remove_empty_dir(&scripts_dir).await?;

		Ok(())
	}
}

/// Errors that can occur when using incremental installs
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when removing unused packages
	#[derive(Debug, Error)]
	pub enum RemoveUnusedError {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// IO error
		#[error("IO error")]
		Io(#[from] std::io::Error),

		/// Removing a patch failed
		#[cfg(feature = "patches")]
		#[error("error removing patch")]
		PatchRemove(#[from] crate::patches::errors::ApplyPatchError),
	}
}
