use crate::PACKAGES_CONTAINER_NAME;
use crate::Project;
use crate::resolver::DependencyGraph;
use crate::source::Realm;
use crate::source::RealmExt as _;
use crate::util::remove_empty_dir;
use fs_err::tokio as fs;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tokio::task::JoinSet;

impl Project {
	/// Removes unused packages from the project
	pub async fn remove_unused(
		&self,
		graph: &DependencyGraph,
	) -> Result<(), errors::RemoveUnusedError> {
		let mut tasks = graph
			.importers
			.keys()
			.flat_map(|importer| {
				[None, Some(Realm::Shared), Some(Realm::Server)]
					.into_iter()
					.map(|realm| (importer.clone(), realm))
			})
			.map(|(importer, realm)| {
				let subproject = self.clone().subproject(importer.clone());
				let packages_dir: Arc<Path> = subproject
					.dependencies_dir()
					.join(realm.packages_dir())
					.into();

				let expected_aliases = graph.importers[&importer]
					.dependencies
					.iter()
					.filter(|(_, (id, _, _))| graph.realm_of(&importer, id) == realm)
					.map(|(alias, _)| alias)
					.cloned()
					.collect::<HashSet<_>>();
				let expected_ids = graph
					.packages_for_importer(&importer, |_, _| true)
					.iter()
					.map(ToString::to_string)
					.collect::<HashSet<_>>();

				async move {
					let mut tasks = JoinSet::<Result<(), errors::RemoveUnusedError>>::new();
					let index_dir: Arc<Path> = packages_dir.join(PACKAGES_CONTAINER_NAME).into();

					{
						let index_dir = index_dir.clone();
						tasks.spawn(async move {
							let mut read_dir = match fs::read_dir(&index_dir).await {
								Ok(entries) => entries,
								Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
								Err(e) => return Err(e.into()),
							};

							let mut tasks = JoinSet::new();

							while let Some(entry) = read_dir.next_entry().await? {
								let file_name = entry.file_name();

								if file_name
									.to_str()
									.is_some_and(|name| expected_ids.contains(name))
								{
									continue;
								}

								let path = entry.path();
								tasks.spawn(async move { fs::remove_dir_all(path).await });
							}

							while let Some(task) = tasks.join_next().await {
								task.unwrap()?;
							}

							Ok(())
						});
					}

					{
						let packages_dir = packages_dir.clone();
						tasks.spawn(async move {
							let mut read_dir = match fs::read_dir(&packages_dir).await {
								Ok(entries) => entries,
								Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
								Err(e) => return Err(e.into()),
							};

							let mut tasks = JoinSet::new();

							while let Some(entry) = read_dir.next_entry().await? {
								let file_name = entry.file_name();
								if file_name == PACKAGES_CONTAINER_NAME {
									continue;
								}

								if file_name.to_str().is_some_and(|name| {
									name.parse()
										.is_ok_and(|alias| expected_aliases.contains(&alias))
								}) {
									continue;
								}

								let path = entry.path();
								tasks.spawn(async move { fs::remove_file(path).await });
							}

							while let Some(task) = tasks.join_next().await {
								task.unwrap()?;
							}

							Ok(())
						});
					}

					while let Some(task) = tasks.join_next().await {
						task.unwrap()?;
					}

					remove_empty_dir(&index_dir).await?;
					remove_empty_dir(&packages_dir).await?;

					Ok::<_, errors::RemoveUnusedError>(())
				}
			})
			.collect::<JoinSet<Result<(), errors::RemoveUnusedError>>>();

		while let Some(task) = tasks.join_next().await {
			task.unwrap()?;
		}

		Ok(())
	}
}

/// Errors that can occur when using incremental installs
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when removing unused packages
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = RemoveUnusedError))]
	pub enum RemoveUnusedErrorKind {
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
