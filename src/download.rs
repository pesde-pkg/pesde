use crate::{
    lockfile::{DependencyGraph, DownloadedDependencyGraphNode},
    manifest::DependencyType,
    names::PackageNames,
    refresh_sources,
    reporters::{DownloadProgressReporter, DownloadsReporter},
    source::{
        traits::{PackageRef, PackageSource},
        version_id::VersionId,
        PackageSources,
    },
    Project, PACKAGES_CONTAINER_NAME,
};
use async_stream::try_stream;
use fs_err::tokio as fs;
use futures::Stream;
use std::{collections::HashSet, num::NonZeroUsize, sync::Arc};
use tokio::{sync::Semaphore, task::JoinSet};
use tracing::{instrument, Instrument};

/// Options for downloading.
#[derive(Debug)]
pub struct DownloadGraphOptions<Reporter> {
    /// The reqwest client.
    pub reqwest: reqwest::Client,
    /// The downloads reporter.
    pub reporter: Option<Arc<Reporter>>,
    /// Whether to skip dev dependencies.
    pub prod: bool,
    /// Whether to write the downloaded packages to disk.
    pub write: bool,
    /// Whether to download Wally packages.
    pub wally: bool,
    /// The max number of concurrent network requests.
    pub network_concurrency: NonZeroUsize,
}

impl<Reporter> DownloadGraphOptions<Reporter>
where
    Reporter: for<'a> DownloadsReporter<'a> + Send + Sync + 'static,
{
    /// Creates a new download options with the given reqwest client and reporter.
    pub fn new(reqwest: reqwest::Client) -> Self {
        Self {
            reqwest,
            reporter: None,
            prod: false,
            write: false,
            wally: false,
            network_concurrency: NonZeroUsize::new(16).unwrap(),
        }
    }

    /// Sets the downloads reporter.
    pub fn reporter(mut self, reporter: impl Into<Arc<Reporter>>) -> Self {
        self.reporter.replace(reporter.into());
        self
    }

    /// Sets whether to skip dev dependencies.
    pub fn prod(mut self, prod: bool) -> Self {
        self.prod = prod;
        self
    }

    /// Sets whether to write the downloaded packages to disk.
    pub fn write(mut self, write: bool) -> Self {
        self.write = write;
        self
    }

    /// Sets whether to download Wally packages.
    pub fn wally(mut self, wally: bool) -> Self {
        self.wally = wally;
        self
    }

    /// Sets the max number of concurrent network requests.
    pub fn network_concurrency(mut self, network_concurrency: NonZeroUsize) -> Self {
        self.network_concurrency = network_concurrency;
        self
    }
}

impl<Reporter> Clone for DownloadGraphOptions<Reporter> {
    fn clone(&self) -> Self {
        Self {
            reqwest: self.reqwest.clone(),
            reporter: self.reporter.clone(),
            prod: self.prod,
            write: self.write,
            wally: self.wally,
            network_concurrency: self.network_concurrency,
        }
    }
}

impl Project {
    /// Downloads a graph of dependencies.
    #[instrument(skip_all, fields(prod = options.prod, wally = options.wally, write = options.write), level = "debug")]
    pub async fn download_graph<Reporter>(
        &self,
        graph: &DependencyGraph,
        refreshed_sources: &mut HashSet<PackageSources>,
        options: DownloadGraphOptions<Reporter>,
    ) -> Result<
        impl Stream<
            Item = Result<
                (DownloadedDependencyGraphNode, PackageNames, VersionId),
                errors::DownloadGraphError,
            >,
        >,
        errors::DownloadGraphError,
    >
    where
        Reporter: for<'a> DownloadsReporter<'a> + Send + Sync + 'static,
    {
        let DownloadGraphOptions {
            reqwest,
            reporter,
            prod,
            write,
            wally,
            network_concurrency,
        } = options;

        let manifest = self.deser_manifest().await?;
        let manifest_target_kind = manifest.target.kind();
        let project = Arc::new(self.clone());

        refresh_sources(
            self,
            graph
                .iter()
                .flat_map(|(_, versions)| versions.iter())
                .map(|(_, node)| node.pkg_ref.source()),
            refreshed_sources,
        )
        .await?;

        let mut tasks = JoinSet::<Result<_, errors::DownloadGraphError>>::new();
        let semaphore = Arc::new(Semaphore::new(network_concurrency.get()));

        for (name, versions) in graph {
            for (version_id, node) in versions {
                // we need to download pesde packages first, since scripts (for target finding for example) can depend on them
                if node.pkg_ref.like_wally() != wally {
                    continue;
                }

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
                let reporter = reporter.clone();
                let package_dir = project.package_dir().to_path_buf();
                let semaphore = semaphore.clone();

                tasks.spawn(
                    async move {
                        let display_name = format!("{name}@{version_id}");
                        let progress_reporter = reporter
                            .as_deref()
                            .map(|reporter| reporter.report_download(&display_name));

                        let _permit = semaphore.acquire().await;

                        if let Some(ref progress_reporter) = progress_reporter {
                            progress_reporter.report_start();
                        }

                        let source = node.pkg_ref.source();
                        let container_folder = node.container_folder(
                            &package_dir
                                .join(manifest_target_kind.packages_folder(version_id.target()))
                                .join(PACKAGES_CONTAINER_NAME),
                            &name,
                            version_id.version(),
                        );

                        fs::create_dir_all(&container_folder).await?;

                        let project = project.clone();

                        tracing::debug!("downloading");

                        let (fs, target) = match progress_reporter {
                            Some(progress_reporter) => {
                                source
                                    .download(
                                        &node.pkg_ref,
                                        &project,
                                        &reqwest,
                                        Arc::new(progress_reporter),
                                    )
                                    .await
                            }
                            None => {
                                source
                                    .download(&node.pkg_ref, &project, &reqwest, Arc::new(()))
                                    .await
                            }
                        }
                        .map_err(Box::new)?;

                        tracing::debug!("downloaded");

                        if write {
                            if !prod || node.resolved_ty != DependencyType::Dev {
                                fs.write_to(container_folder, project.cas_dir(), true)
                                    .await?;
                            } else {
                                tracing::debug!(
                                    "skipping write to disk, dev dependency in prod mode"
                                );
                            }
                        }

                        let downloaded_node = DownloadedDependencyGraphNode { node, target };
                        Ok((downloaded_node, name, version_id))
                    }
                    .instrument(span),
                );
            }
        }

        let stream = try_stream! {
            while let Some(res) = tasks.join_next().await {
                yield res.unwrap()?;
            }
        };

        Ok(stream)
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
