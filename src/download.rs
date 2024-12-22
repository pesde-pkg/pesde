use crate::{
    lockfile::{DependencyGraph, DownloadedDependencyGraphNode, DownloadedGraph},
    manifest::DependencyType,
    refresh_sources,
    source::{
        traits::{PackageRef, PackageSource},
        PackageSources,
    },
    Project, PACKAGES_CONTAINER_NAME,
};
use fs_err::tokio as fs;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tracing::{instrument, Instrument};

type MultithreadedGraph = Arc<Mutex<DownloadedGraph>>;

pub(crate) type MultithreadDownloadJob = (
    tokio::sync::mpsc::Receiver<Result<String, errors::DownloadGraphError>>,
    MultithreadedGraph,
);

impl Project {
    /// Downloads a graph of dependencies
    #[instrument(skip(self, graph, refreshed_sources, reqwest), level = "debug")]
    pub async fn download_graph(
        &self,
        graph: &DependencyGraph,
        refreshed_sources: &mut HashSet<PackageSources>,
        reqwest: &reqwest::Client,
        prod: bool,
        write: bool,
        wally: bool,
    ) -> Result<MultithreadDownloadJob, errors::DownloadGraphError> {
        let manifest = self.deser_manifest().await?;
        let manifest_target_kind = manifest.target.kind();
        let downloaded_graph: MultithreadedGraph = Arc::new(Mutex::new(Default::default()));

        let (tx, rx) = tokio::sync::mpsc::channel(
            graph
                .iter()
                .map(|(_, versions)| versions.len())
                .sum::<usize>()
                .max(1),
        );

        refresh_sources(
            self,
            graph
                .iter()
                .flat_map(|(_, versions)| versions.iter())
                .map(|(_, node)| node.pkg_ref.source()),
            refreshed_sources,
        )
        .await?;

        let project = Arc::new(self.clone());

        for (name, versions) in graph {
            for (version_id, node) in versions {
                // we need to download pesde packages first, since scripts (for target finding for example) can depend on them
                if node.pkg_ref.like_wally() != wally {
                    continue;
                }

                let tx = tx.clone();

                let name = name.clone();
                let version_id = version_id.clone();
                let node = node.clone();

                let span = tracing::info_span!(
                    "download",
                    name = name.to_string(),
                    version_id = version_id.to_string()
                );

                let project = project.clone();
                let reqwest = reqwest.clone();
                let downloaded_graph = downloaded_graph.clone();

                let package_dir = self.package_dir().to_path_buf();

                tokio::spawn(
                    async move {
                        let source = node.pkg_ref.source();

                        let container_folder = node.container_folder(
                            &package_dir
                                .join(manifest_target_kind.packages_folder(version_id.target()))
                                .join(PACKAGES_CONTAINER_NAME),
                            &name,
                            version_id.version(),
                        );

                        match fs::create_dir_all(&container_folder).await {
                            Ok(_) => {}
                            Err(e) => {
                                tx.send(Err(errors::DownloadGraphError::Io(e)))
                                    .await
                                    .unwrap();
                                return;
                            }
                        }

                        let project = project.clone();

                        tracing::debug!("downloading");

                        let (fs, target) =
                            match source.download(&node.pkg_ref, &project, &reqwest).await {
                                Ok(target) => target,
                                Err(e) => {
                                    tx.send(Err(Box::new(e).into())).await.unwrap();
                                    return;
                                }
                            };

                        tracing::debug!("downloaded");

                        if write {
                            if !prod || node.resolved_ty != DependencyType::Dev {
                                match fs.write_to(container_folder, project.cas_dir(), true).await {
                                    Ok(_) => {}
                                    Err(e) => {
                                        tx.send(Err(errors::DownloadGraphError::WriteFailed(e)))
                                            .await
                                            .unwrap();
                                        return;
                                    }
                                };
                            } else {
                                tracing::debug!(
                                    "skipping write to disk, dev dependency in prod mode"
                                );
                            }
                        }

                        let display_name = format!("{name}@{version_id}");

                        {
                            let mut downloaded_graph = downloaded_graph.lock().unwrap();
                            downloaded_graph
                                .entry(name)
                                .or_default()
                                .insert(version_id, DownloadedDependencyGraphNode { node, target });
                        }

                        tx.send(Ok(display_name)).await.unwrap();
                    }
                    .instrument(span),
                );
            }
        }

        Ok((rx, downloaded_graph))
    }
}

/// Errors that can occur when downloading a graph
pub mod errors {
    use thiserror::Error;

    /// Errors that can occur when downloading a graph
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum DownloadGraphError {
        /// An error occurred deserializing the project manifest
        #[error("error deserializing project manifest")]
        ManifestDeserializationFailed(#[from] crate::errors::ManifestReadError),

        /// An error occurred refreshing a package source
        #[error("failed to refresh package source")]
        RefreshFailed(#[from] Box<crate::source::errors::RefreshError>),

        /// Error interacting with the filesystem
        #[error("error interacting with the filesystem")]
        Io(#[from] std::io::Error),

        /// Error downloading a package
        #[error("failed to download package")]
        DownloadFailed(#[from] Box<crate::source::errors::DownloadError>),

        /// Error writing package contents
        #[error("failed to write package contents")]
        WriteFailed(#[source] std::io::Error),
    }
}
