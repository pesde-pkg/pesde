use crate::{lockfile::DownloadedGraph, Project, MANIFEST_FILE_NAME, PACKAGES_CONTAINER_NAME};
use fs_err::tokio as fs;
use git2::{ApplyLocation, Diff, DiffFormat, DiffLineType, Repository, Signature};
use relative_path::RelativePathBuf;
use std::path::Path;
use tracing::instrument;

/// Set up a git repository for patches
pub fn setup_patches_repo<P: AsRef<Path>>(dir: P) -> Result<Repository, git2::Error> {
    let repo = Repository::init(&dir)?;

    {
        let signature = Signature::now(
            env!("CARGO_PKG_NAME"),
            concat!(env!("CARGO_PKG_NAME"), "@localhost"),
        )?;
        let mut index = repo.index()?;
        index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;

        let oid = index.write_tree()?;
        let tree = repo.find_tree(oid)?;

        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "begin patch",
            &tree,
            &[],
        )?;
    }

    Ok(repo)
}

/// Create a patch from the current state of the repository
pub fn create_patch<P: AsRef<Path>>(dir: P) -> Result<Vec<u8>, git2::Error> {
    let mut patches = vec![];
    let repo = Repository::open(dir.as_ref())?;

    let original = repo.head()?.peel_to_tree()?;

    // reset the manifest file to the original state
    let mut checkout_builder = git2::build::CheckoutBuilder::new();
    checkout_builder.force();
    checkout_builder.path(MANIFEST_FILE_NAME);
    repo.checkout_tree(original.as_object(), Some(&mut checkout_builder))?;

    let diff = repo.diff_tree_to_workdir(Some(&original), None)?;

    diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        if matches!(
            line.origin_value(),
            DiffLineType::Context | DiffLineType::Addition | DiffLineType::Deletion
        ) {
            let origin = line.origin();
            let mut buffer = vec![0; origin.len_utf8()];
            origin.encode_utf8(&mut buffer);
            patches.extend(buffer);
        }

        patches.extend(line.content());

        true
    })?;

    Ok(patches)
}

impl Project {
    /// Apply patches to the project's dependencies
    #[instrument(skip(self, graph), level = "debug")]
    pub async fn apply_patches(
        &self,
        graph: &DownloadedGraph,
    ) -> Result<
        tokio::sync::mpsc::Receiver<Result<String, errors::ApplyPatchesError>>,
        errors::ApplyPatchesError,
    > {
        let manifest = self.deser_manifest().await?;
        let (tx, rx) = tokio::sync::mpsc::channel(
            manifest
                .patches
                .values()
                .map(|v| v.len())
                .sum::<usize>()
                .max(1),
        );

        for (name, versions) in manifest.patches {
            for (version_id, patch_path) in versions {
                let tx = tx.clone();

                let name = name.clone();
                let patch_path = patch_path.to_path(self.package_dir());

                let Some(node) = graph
                    .get(&name)
                    .and_then(|versions| versions.get(&version_id))
                else {
                    tracing::warn!(
                        "patch for {name}@{version_id} not applied because it is not in the graph"
                    );
                    tx.send(Ok(format!("{name}@{version_id}"))).await.unwrap();
                    continue;
                };

                let container_folder = node.node.container_folder(
                    &self
                        .package_dir()
                        .join(manifest.target.kind().packages_folder(version_id.target()))
                        .join(PACKAGES_CONTAINER_NAME),
                    &name,
                    version_id.version(),
                );

                tokio::spawn(async move {
                    tracing::debug!("applying patch to {name}@{version_id}");

                    let patch = match fs::read(&patch_path).await {
                        Ok(patch) => patch,
                        Err(e) => {
                            tx.send(Err(errors::ApplyPatchesError::PatchRead(e)))
                                .await
                                .unwrap();
                            return;
                        }
                    };

                    let patch = match Diff::from_buffer(&patch) {
                        Ok(patch) => patch,
                        Err(e) => {
                            tx.send(Err(errors::ApplyPatchesError::Git(e)))
                                .await
                                .unwrap();
                            return;
                        }
                    };

                    {
                        let repo = match setup_patches_repo(&container_folder) {
                            Ok(repo) => repo,
                            Err(e) => {
                                tx.send(Err(errors::ApplyPatchesError::Git(e)))
                                    .await
                                    .unwrap();
                                return;
                            }
                        };

                        let modified_files = patch
                            .deltas()
                            .filter(|delta| matches!(delta.status(), git2::Delta::Modified))
                            .filter_map(|delta| delta.new_file().path())
                            .map(|path| {
                                RelativePathBuf::from_path(path)
                                    .unwrap()
                                    .to_path(&container_folder)
                            })
                            .filter(|path| path.is_file())
                            .collect::<Vec<_>>();

                        for path in modified_files {
                            // there is no way (as far as I know) to check if it's hardlinked
                            // so, we always unlink it
                            let content = match fs::read(&path).await {
                                Ok(content) => content,
                                Err(e) => {
                                    tx.send(Err(errors::ApplyPatchesError::File(e)))
                                        .await
                                        .unwrap();
                                    return;
                                }
                            };

                            if let Err(e) = fs::remove_file(&path).await {
                                tx.send(Err(errors::ApplyPatchesError::File(e)))
                                    .await
                                    .unwrap();
                                return;
                            }

                            if let Err(e) = fs::write(path, content).await {
                                tx.send(Err(errors::ApplyPatchesError::File(e)))
                                    .await
                                    .unwrap();
                                return;
                            }
                        }

                        if let Err(e) = repo.apply(&patch, ApplyLocation::Both, None) {
                            tx.send(Err(errors::ApplyPatchesError::Git(e)))
                                .await
                                .unwrap();
                            return;
                        }
                    }

                    tracing::debug!(
                        "patch applied to {name}@{version_id}, removing .git directory"
                    );

                    if let Err(e) = fs::remove_dir_all(container_folder.join(".git")).await {
                        tx.send(Err(errors::ApplyPatchesError::DotGitRemove(e)))
                            .await
                            .unwrap();
                        return;
                    }

                    tx.send(Ok(format!("{name}@{version_id}"))).await.unwrap();
                });
            }
        }

        Ok(rx)
    }
}

/// Errors that can occur when using patches
pub mod errors {
    use thiserror::Error;

    /// Errors that can occur when applying patches
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum ApplyPatchesError {
        /// Error deserializing the project manifest
        #[error("error deserializing project manifest")]
        ManifestDeserializationFailed(#[from] crate::errors::ManifestReadError),

        /// Error interacting with git
        #[error("error interacting with git")]
        Git(#[from] git2::Error),

        /// Error reading the patch file
        #[error("error reading patch file")]
        PatchRead(#[source] std::io::Error),

        /// Error removing the .git directory
        #[error("error removing .git directory")]
        DotGitRemove(#[source] std::io::Error),

        /// Error interacting with a patched file
        #[error("error interacting with a patched file")]
        File(#[source] std::io::Error),
    }
}
