use crate::{
    download::DownloadGraphOptions,
    lockfile::{DependencyGraph, DownloadedGraph},
    manifest::DependencyType,
    reporters::DownloadsReporter,
    source::PackageSources,
    Project,
};
use futures::TryStreamExt;
use std::{
    collections::HashSet,
    convert::Infallible,
    future::{self, Future},
    num::NonZeroUsize,
    sync::Arc,
};
use tokio::sync::Mutex;
use tracing::{instrument, Instrument};

/// Filters a graph to only include production dependencies, if `prod` is `true`
pub fn filter_graph(graph: &DownloadedGraph, prod: bool) -> DownloadedGraph {
    if !prod {
        return graph.clone();
    }

    graph
        .iter()
        .map(|(name, versions)| {
            (
                name.clone(),
                versions
                    .iter()
                    .filter(|(_, node)| node.node.resolved_ty != DependencyType::Dev)
                    .map(|(v_id, node)| (v_id.clone(), node.clone()))
                    .collect(),
            )
        })
        .collect()
}

/// Receiver for dependencies downloaded and linked
pub type DownloadAndLinkReceiver =
    tokio::sync::mpsc::Receiver<Result<String, crate::download::errors::DownloadGraphError>>;

/// Hooks to perform actions after certain events during download and linking.
#[allow(unused_variables)]
pub trait DownloadAndLinkHooks {
    /// The error type for the hooks.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Called after scripts have been downloaded. The `downloaded_graph`
    /// contains all downloaded packages.
    fn on_scripts_downloaded(
        &self,
        downloaded_graph: &DownloadedGraph,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        future::ready(Ok(()))
    }

    /// Called after binary dependencies have been downloaded. The
    /// `downloaded_graph` contains all downloaded packages.
    fn on_bins_downloaded(
        &self,
        downloaded_graph: &DownloadedGraph,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        future::ready(Ok(()))
    }

    /// Called after all dependencies have been downloaded. The
    /// `downloaded_graph` contains all downloaded packages.
    fn on_all_downloaded(
        &self,
        downloaded_graph: &DownloadedGraph,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        future::ready(Ok(()))
    }
}

impl DownloadAndLinkHooks for () {
    type Error = Infallible;
}

/// Options for downloading and linking.
#[derive(Debug)]
pub struct DownloadAndLinkOptions<Reporter = (), Hooks = ()> {
    /// The reqwest client.
    pub reqwest: reqwest::Client,
    /// The downloads reporter.
    pub reporter: Option<Arc<Reporter>>,
    /// The download and link hooks.
    pub hooks: Option<Arc<Hooks>>,
    /// The refreshed sources.
    pub refreshed_sources: Arc<Mutex<HashSet<PackageSources>>>,
    /// Whether to skip dev dependencies.
    pub prod: bool,
    /// Whether to write the downloaded packages to disk.
    pub write: bool,
    /// The max number of concurrent network requests.
    pub network_concurrency: NonZeroUsize,
}

impl<Reporter, Hooks> DownloadAndLinkOptions<Reporter, Hooks>
where
    Reporter: for<'a> DownloadsReporter<'a> + Send + Sync + 'static,
    Hooks: DownloadAndLinkHooks + Send + Sync + 'static,
{
    /// Creates a new download options with the given reqwest client and reporter.
    pub fn new(reqwest: reqwest::Client) -> Self {
        Self {
            reqwest,
            reporter: None,
            hooks: None,
            refreshed_sources: Default::default(),
            prod: false,
            write: true,
            network_concurrency: NonZeroUsize::new(16).unwrap(),
        }
    }

    /// Sets the downloads reporter.
    pub fn reporter(mut self, reporter: impl Into<Arc<Reporter>>) -> Self {
        self.reporter.replace(reporter.into());
        self
    }

    /// Sets the download and link hooks.
    pub fn hooks(mut self, hooks: impl Into<Arc<Hooks>>) -> Self {
        self.hooks.replace(hooks.into());
        self
    }

    /// Sets the refreshed sources.
    pub fn refreshed_sources(
        mut self,
        refreshed_sources: impl Into<Arc<Mutex<HashSet<PackageSources>>>>,
    ) -> Self {
        self.refreshed_sources = refreshed_sources.into();
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

    /// Sets the max number of concurrent network requests.
    pub fn network_concurrency(mut self, network_concurrency: NonZeroUsize) -> Self {
        self.network_concurrency = network_concurrency;
        self
    }
}

impl Clone for DownloadAndLinkOptions {
    fn clone(&self) -> Self {
        Self {
            reqwest: self.reqwest.clone(),
            reporter: self.reporter.clone(),
            hooks: self.hooks.clone(),
            refreshed_sources: self.refreshed_sources.clone(),
            prod: self.prod,
            write: self.write,
            network_concurrency: self.network_concurrency,
        }
    }
}

impl Project {
    /// Downloads a graph of dependencies and links them in the correct order
    #[instrument(skip_all, fields(prod = options.prod, write = options.write), level = "debug")]
    pub async fn download_and_link<Reporter, Hooks>(
        &self,
        graph: &Arc<DependencyGraph>,
        options: DownloadAndLinkOptions<Reporter, Hooks>,
    ) -> Result<DownloadedGraph, errors::DownloadAndLinkError<Hooks::Error>>
    where
        Reporter: for<'a> DownloadsReporter<'a> + 'static,
        Hooks: DownloadAndLinkHooks + 'static,
    {
        let DownloadAndLinkOptions {
            reqwest,
            reporter,
            hooks,
            refreshed_sources,
            prod,
            write,
            network_concurrency,
        } = options;

        let graph = graph.clone();
        let reqwest = reqwest.clone();

        let mut refreshed_sources = refreshed_sources.lock().await;
        let mut downloaded_graph = DownloadedGraph::new();

        let mut download_graph_options = DownloadGraphOptions::<Reporter>::new(reqwest.clone())
            .prod(prod)
            .write(write)
            .network_concurrency(network_concurrency);

        if let Some(reporter) = reporter {
            download_graph_options = download_graph_options.reporter(reporter.clone());
        }

        // step 1. download pesde dependencies
        self.download_graph(
            &graph,
            &mut refreshed_sources,
            download_graph_options.clone(),
        )
        .instrument(tracing::debug_span!("download (pesde)"))
        .await?
        .try_for_each(|(downloaded_node, name, version_id)| {
            downloaded_graph
                .entry(name)
                .or_default()
                .insert(version_id, downloaded_node);

            future::ready(Ok(()))
        })
        .await?;

        // step 2. link pesde dependencies. do so without types
        if write {
            self.link_dependencies(&filter_graph(&downloaded_graph, prod), false)
                .instrument(tracing::debug_span!("link (pesde)"))
                .await?;
        }

        if let Some(ref hooks) = hooks {
            hooks
                .on_scripts_downloaded(&downloaded_graph)
                .await
                .map_err(errors::DownloadAndLinkError::Hook)?;

            hooks
                .on_bins_downloaded(&downloaded_graph)
                .await
                .map_err(errors::DownloadAndLinkError::Hook)?;
        }

        // step 3. download wally dependencies
        self.download_graph(
            &graph,
            &mut refreshed_sources,
            download_graph_options.clone().wally(true),
        )
        .instrument(tracing::debug_span!("download (wally)"))
        .await?
        .try_for_each(|(downloaded_node, name, version_id)| {
            downloaded_graph
                .entry(name)
                .or_default()
                .insert(version_id, downloaded_node);

            future::ready(Ok(()))
        })
        .await?;

        // step 4. link ALL dependencies. do so with types
        if write {
            self.link_dependencies(&filter_graph(&downloaded_graph, prod), true)
                .instrument(tracing::debug_span!("link (all)"))
                .await?;
        }

        if let Some(ref hooks) = hooks {
            hooks
                .on_all_downloaded(&downloaded_graph)
                .await
                .map_err(errors::DownloadAndLinkError::Hook)?;
        }

        Ok(downloaded_graph)
    }
}

/// Errors that can occur when downloading and linking dependencies
pub mod errors {
    use thiserror::Error;

    /// An error that can occur when downloading and linking dependencies
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum DownloadAndLinkError<E> {
        /// An error occurred while downloading the graph
        #[error("error downloading graph")]
        DownloadGraph(#[from] crate::download::errors::DownloadGraphError),

        /// An error occurred while linking dependencies
        #[error("error linking dependencies")]
        Linking(#[from] crate::linking::errors::LinkingError),

        /// An error occurred while executing the pesde callback
        #[error("error executing hook")]
        Hook(#[source] E),
    }
}
