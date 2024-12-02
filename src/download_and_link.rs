use crate::{
    lockfile::{DependencyGraph, DownloadedGraph},
    manifest::DependencyType,
    source::PackageSources,
    Project,
};
use futures::FutureExt;
use std::{
    collections::HashSet,
    future::Future,
    sync::{Arc, Mutex as StdMutex},
};
use tokio::sync::Mutex;

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

impl Project {
    /// Downloads a graph of dependencies and links them in the correct order
    pub async fn download_and_link<
        F: FnOnce(&Arc<DownloadedGraph>) -> R + Send + 'static,
        R: Future<Output = Result<(), E>> + Send,
        E: Send + Sync + 'static,
    >(
        &self,
        graph: &Arc<DependencyGraph>,
        refreshed_sources: &Arc<Mutex<HashSet<PackageSources>>>,
        reqwest: &reqwest::Client,
        prod: bool,
        write: bool,
        pesde_cb: F,
    ) -> Result<
        (
            tokio::sync::mpsc::Receiver<
                Result<String, crate::download::errors::DownloadGraphError>,
            >,
            impl Future<Output = Result<DownloadedGraph, errors::DownloadAndLinkError<E>>>,
        ),
        errors::DownloadAndLinkError<E>,
    > {
        let (tx, rx) = tokio::sync::mpsc::channel(
            graph
                .iter()
                .map(|(_, versions)| versions.len())
                .sum::<usize>()
                .max(1),
        );
        let downloaded_graph = Arc::new(StdMutex::new(DownloadedGraph::default()));

        let this = self.clone();
        let graph = graph.clone();
        let reqwest = reqwest.clone();
        let refreshed_sources = refreshed_sources.clone();

        Ok((
            rx,
            tokio::spawn(async move {
                let mut refreshed_sources = refreshed_sources.lock().await;

                // step 1. download pesde dependencies
                let (mut pesde_rx, pesde_graph) = this
                    .download_graph(&graph, &mut refreshed_sources, &reqwest, prod, write, false)
                    .await?;

                while let Some(result) = pesde_rx.recv().await {
                    tx.send(result).await.unwrap();
                }

                let pesde_graph = Arc::into_inner(pesde_graph).unwrap().into_inner().unwrap();

                // step 2. link pesde dependencies. do so without types
                if write {
                    this.link_dependencies(&filter_graph(&pesde_graph, prod), false)
                        .await?;
                }

                let pesde_graph = Arc::new(pesde_graph);

                pesde_cb(&pesde_graph)
                    .await
                    .map_err(errors::DownloadAndLinkError::PesdeCallback)?;

                let pesde_graph = Arc::into_inner(pesde_graph).unwrap();

                // step 3. download wally dependencies
                let (mut wally_rx, wally_graph) = this
                    .download_graph(&graph, &mut refreshed_sources, &reqwest, prod, write, true)
                    .await?;

                while let Some(result) = wally_rx.recv().await {
                    tx.send(result).await.unwrap();
                }

                let wally_graph = Arc::into_inner(wally_graph).unwrap().into_inner().unwrap();

                {
                    let mut downloaded_graph = downloaded_graph.lock().unwrap();
                    downloaded_graph.extend(pesde_graph);
                    for (name, versions) in wally_graph {
                        for (version_id, node) in versions {
                            downloaded_graph
                                .entry(name.clone())
                                .or_default()
                                .insert(version_id, node);
                        }
                    }
                }

                let graph = Arc::into_inner(downloaded_graph)
                    .unwrap()
                    .into_inner()
                    .unwrap();

                // step 4. link ALL dependencies. do so with types
                if write {
                    this.link_dependencies(&filter_graph(&graph, prod), true)
                        .await?;
                }

                Ok(graph)
            })
            .map(|r| r.unwrap()),
        ))
    }
}

/// Errors that can occur when downloading and linking dependencies
pub mod errors {
    use thiserror::Error;

    /// An error that can occur when downloading and linking dependencies
    #[derive(Debug, Error)]
    pub enum DownloadAndLinkError<E> {
        /// An error occurred while downloading the graph
        #[error("error downloading graph")]
        DownloadGraph(#[from] crate::download::errors::DownloadGraphError),

        /// An error occurred while linking dependencies
        #[error("error linking dependencies")]
        Linking(#[from] crate::linking::errors::LinkingError),

        /// An error occurred while executing the pesde callback
        #[error("error executing pesde callback")]
        PesdeCallback(#[source] E),
    }
}
