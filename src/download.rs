use crate::{
	graph::{DependencyGraph, DependencyGraphNode},
	reporters::{DownloadProgressReporter as _, DownloadsReporter},
	source::{
		fs::PackageFs,
		ids::PackageId,
		traits::{DownloadOptions, PackageRef as _, PackageSource as _, RefreshOptions},
	},
	Project, RefreshedSources,
};
use async_stream::try_stream;
use futures::Stream;
use std::{num::NonZeroUsize, sync::Arc};
use tokio::{sync::Semaphore, task::JoinSet};
use tracing::{instrument, Instrument as _};

/// Options for downloading.
#[derive(Debug)]
pub(crate) struct DownloadGraphOptions<Reporter> {
	/// The reqwest client.
	pub reqwest: reqwest::Client,
	/// The downloads reporter.
	pub reporter: Option<Arc<Reporter>>,
	/// The refreshed sources.
	pub refreshed_sources: RefreshedSources,
	/// The max number of concurrent network requests.
	pub network_concurrency: NonZeroUsize,
}

impl<Reporter> DownloadGraphOptions<Reporter>
where
	Reporter: DownloadsReporter + Send + Sync + 'static,
{
	/// Creates a new download options with the given reqwest client and reporter.
	pub(crate) fn new(reqwest: reqwest::Client) -> Self {
		Self {
			reqwest,
			reporter: None,
			refreshed_sources: Default::default(),
			network_concurrency: NonZeroUsize::new(16).unwrap(),
		}
	}

	/// Sets the downloads reporter.
	pub(crate) fn reporter(mut self, reporter: impl Into<Arc<Reporter>>) -> Self {
		self.reporter.replace(reporter.into());
		self
	}

	/// Sets the refreshed sources.
	pub(crate) fn refreshed_sources(mut self, refreshed_sources: RefreshedSources) -> Self {
		self.refreshed_sources = refreshed_sources;
		self
	}

	/// Sets the max number of concurrent network requests.
	pub(crate) fn network_concurrency(mut self, network_concurrency: NonZeroUsize) -> Self {
		self.network_concurrency = network_concurrency;
		self
	}
}

impl<Reporter> Clone for DownloadGraphOptions<Reporter> {
	fn clone(&self) -> Self {
		Self {
			reqwest: self.reqwest.clone(),
			reporter: self.reporter.clone(),
			refreshed_sources: self.refreshed_sources.clone(),
			network_concurrency: self.network_concurrency,
		}
	}
}

impl Project {
	/// Downloads a graph of dependencies.
	#[instrument(skip_all, level = "debug")]
	pub(crate) fn download_graph<Reporter>(
		&self,
		graph: &DependencyGraph,
		options: DownloadGraphOptions<Reporter>,
	) -> Result<
		impl Stream<
			Item = Result<(PackageId, DependencyGraphNode, PackageFs), errors::DownloadGraphError>,
		>,
		errors::DownloadGraphError,
	>
	where
		Reporter: DownloadsReporter + Send + Sync + 'static,
	{
		let DownloadGraphOptions {
			reqwest,
			reporter,
			refreshed_sources,
			network_concurrency,
		} = options;

		let semaphore = Arc::new(Semaphore::new(network_concurrency.get()));

		let mut tasks = graph
			.iter()
			.map(|(package_id, node)| {
				let span = tracing::info_span!("download", package_id = package_id.to_string());

				let project = self.clone();
				let reqwest = reqwest.clone();
				let reporter = reporter.clone();
				let refreshed_sources = refreshed_sources.clone();
				let semaphore = semaphore.clone();
				let package_id = Arc::new(package_id.clone());
				let node = node.clone();

				async move {
					let progress_reporter = reporter
						.clone()
						.map(|reporter| reporter.report_download(package_id.to_string()));

					let _permit = semaphore.acquire().await;

					if let Some(progress_reporter) = &progress_reporter {
						progress_reporter.report_start();
					}

					let source = node.pkg_ref.source();
					refreshed_sources
						.refresh(
							&source,
							&RefreshOptions {
								project: project.clone(),
							},
						)
						.await?;

					tracing::debug!("downloading");

					let fs = match progress_reporter {
						Some(progress_reporter) => {
							source
								.download(
									&node.pkg_ref,
									&DownloadOptions {
										project: project.clone(),
										reqwest,
										id: package_id.clone(),
										reporter: progress_reporter.into(),
									},
								)
								.await
						}
						None => {
							source
								.download(
									&node.pkg_ref,
									&DownloadOptions {
										project: project.clone(),
										reqwest,
										id: package_id.clone(),
										reporter: ().into(),
									},
								)
								.await
						}
					}
					.map_err(Box::new)?;

					tracing::debug!("downloaded");

					Ok((Arc::into_inner(package_id).unwrap(), node, fs))
				}
				.instrument(span)
			})
			.collect::<JoinSet<Result<_, errors::DownloadGraphError>>>();

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
		/// An error occurred refreshing a package source
		#[error("failed to refresh package source")]
		RefreshFailed(#[from] crate::source::errors::RefreshError),

		/// Error interacting with the filesystem
		#[error("error interacting with the filesystem")]
		Io(#[from] std::io::Error),

		/// Error downloading a package
		#[error("failed to download package")]
		DownloadFailed(#[from] Box<crate::source::errors::DownloadError>),
	}
}
