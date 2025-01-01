use crate::{
    graph::DependencyGraph,
    reporters::{PatchProgressReporter, PatchesReporter},
    source::ids::PackageId,
    Project, MANIFEST_FILE_NAME, PACKAGES_CONTAINER_NAME,
};
use fs_err::tokio as fs;
use futures::TryFutureExt;
use git2::{ApplyLocation, Diff, DiffFormat, DiffLineType, Repository, Signature};
use relative_path::RelativePathBuf;
use std::{path::Path, sync::Arc};
use tokio::task::JoinSet;
use tracing::{instrument, Instrument};

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
    #[instrument(skip(self, graph, reporter), level = "debug")]
    pub async fn apply_patches<Reporter>(
        &self,
        graph: &DependencyGraph,
        reporter: Arc<Reporter>,
    ) -> Result<(), errors::ApplyPatchesError>
    where
        Reporter: for<'a> PatchesReporter<'a> + Send + Sync + 'static,
    {
        let manifest = self.deser_manifest().await?;

        let mut tasks = JoinSet::<Result<_, errors::ApplyPatchesError>>::new();

        for (name, versions) in manifest.patches {
            for (version_id, patch_path) in versions {
                let patch_path = patch_path.to_path(self.package_dir());

                let package_id = PackageId::new(name.clone(), version_id);
                let Some(node) = graph.get(&package_id) else {
                    tracing::warn!(
                        "patch for {package_id} not applied because it is not in the graph"
                    );
                    continue;
                };

                let container_folder = node.container_folder(
                    &self
                        .package_dir()
                        .join(
                            manifest
                                .target
                                .kind()
                                .packages_folder(package_id.version_id().target()),
                        )
                        .join(PACKAGES_CONTAINER_NAME),
                    &package_id,
                );

                let reporter = reporter.clone();
                let span = tracing::info_span!("apply patch", package_id = package_id.to_string());

                tasks.spawn(
                    async move {
                        tracing::debug!("applying patch");

                        let progress_reporter = reporter.report_patch(&package_id.to_string());

                        let patch = fs::read(&patch_path)
                            .await
                            .map_err(errors::ApplyPatchesError::PatchRead)?;
                        let patch = Diff::from_buffer(&patch)?;

                        {
                            let repo = setup_patches_repo(&container_folder)?;

                            let mut apply_delta_tasks = patch
                                .deltas()
                                .filter(|delta| matches!(delta.status(), git2::Delta::Modified))
                                .filter_map(|delta| delta.new_file().path())
                                .map(|path| {
                                    RelativePathBuf::from_path(path)
                                        .unwrap()
                                        .to_path(&container_folder)
                                })
                                .filter(|path| path.is_file())
                                .map(|path| {
                                    async {
                                        // so, we always unlink it
                                        let content = fs::read(&path).await?;
                                        fs::remove_file(&path).await?;
                                        fs::write(path, content).await?;
                                        Ok(())
                                    }
                                    .map_err(errors::ApplyPatchesError::File)
                                })
                                .collect::<JoinSet<_>>();

                            while let Some(res) = apply_delta_tasks.join_next().await {
                                res.unwrap()?;
                            }

                            repo.apply(&patch, ApplyLocation::Both, None)?;
                        }

                        tracing::debug!("patch applied");

                        fs::remove_dir_all(container_folder.join(".git"))
                            .await
                            .map_err(errors::ApplyPatchesError::DotGitRemove)?;

                        progress_reporter.report_done();

                        Ok(())
                    }
                    .instrument(span),
                );
            }
        }

        while let Some(res) = tasks.join_next().await {
            res.unwrap()?
        }

        Ok(())
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
