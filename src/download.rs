use crate::{
    lockfile::{DependencyGraph, DownloadedDependencyGraphNode, DownloadedGraph},
    manifest::DependencyType,
    source::{
        traits::{PackageRef, PackageSource},
        PackageSources,
    },
    Project, PACKAGES_CONTAINER_NAME,
};
use fs_err::create_dir_all;
use std::{
    collections::HashSet,
    sync::{mpsc::Receiver, Arc, Mutex},
};

type MultithreadedGraph = Arc<Mutex<DownloadedGraph>>;

type MultithreadDownloadJob = (
    Receiver<Result<(), errors::DownloadGraphError>>,
    MultithreadedGraph,
);

impl Project {
    /// Downloads a graph of dependencies
    pub fn download_graph(
        &self,
        graph: &DependencyGraph,
        refreshed_sources: &mut HashSet<PackageSources>,
        reqwest: &reqwest::blocking::Client,
        threads: usize,
        prod: bool,
        write: bool,
    ) -> Result<MultithreadDownloadJob, errors::DownloadGraphError> {
        let manifest = self.deser_manifest()?;
        let downloaded_graph: MultithreadedGraph = Arc::new(Mutex::new(Default::default()));

        let threadpool = threadpool::ThreadPool::new(threads);
        let (tx, rx) = std::sync::mpsc::channel();

        for (name, versions) in graph {
            for (version_id, node) in versions {
                let source = node.pkg_ref.source();

                if refreshed_sources.insert(source.clone()) {
                    source.refresh(self).map_err(Box::new)?;
                }

                let container_folder = node.container_folder(
                    &self
                        .package_dir()
                        .join(
                            manifest
                                .target
                                .kind()
                                .packages_folder(&node.pkg_ref.target_kind()),
                        )
                        .join(PACKAGES_CONTAINER_NAME),
                    name,
                    version_id.version(),
                );

                create_dir_all(&container_folder)?;

                let tx = tx.clone();

                let name = name.clone();
                let version_id = version_id.clone();
                let node = node.clone();

                let project = Arc::new(self.clone());
                let reqwest = reqwest.clone();
                let downloaded_graph = downloaded_graph.clone();

                threadpool.execute(move || {
                    let project = project.clone();

                    log::debug!("downloading {name}@{version_id}");

                    let (fs, target) = match source.download(&node.pkg_ref, &project, &reqwest) {
                        Ok(target) => target,
                        Err(e) => {
                            tx.send(Err(Box::new(e).into())).unwrap();
                            return;
                        }
                    };

                    log::debug!("downloaded {name}@{version_id}");

                    if write {
                        if !prod || node.ty != DependencyType::Dev {
                            match fs.write_to(container_folder, project.cas_dir(), true) {
                                Ok(_) => {}
                                Err(e) => {
                                    tx.send(Err(errors::DownloadGraphError::WriteFailed(e)))
                                        .unwrap();
                                    return;
                                }
                            };
                        } else {
                            log::debug!("skipping writing {name}@{version_id} to disk, dev dependency in prod mode");
                        }
                    }

                    let mut downloaded_graph = downloaded_graph.lock().unwrap();
                    downloaded_graph
                        .entry(name)
                        .or_default()
                        .insert(version_id, DownloadedDependencyGraphNode { node, target });

                    tx.send(Ok(())).unwrap();
                });
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
        WriteFailed(std::io::Error),
    }
}
