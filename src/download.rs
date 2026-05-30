//! Downloading packages
use crate::Project;
use crate::RefreshedSources;
use crate::lockfile::Lockfile;
use crate::reporters::DownloadProgressReporter as _;
use crate::reporters::DownloadsReporter;
use crate::source::PackageSource as _;
use crate::source::fs::PackageFs;
use crate::source::ids::PackageId;
use async_stream::try_stream;
use futures::Stream;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::Instrument as _;
use tracing::instrument;

impl Project {
	/// Downloads a graph of dependencies.
	#[instrument(skip_all, level = "debug")]
	pub(crate) fn download_graph<'a, Reporter>(
		&self,
		lockfile: &'a Lockfile,
		package_ids: impl Iterator<Item = &'a PackageId> + 'a,
		reporter: Option<&Arc<Reporter>>,
		refreshed_sources: &RefreshedSources,
		network_concurrency: NonZeroUsize,
	) -> Result<
		impl Stream<Item = Result<(PackageId, PackageFs), errors::DownloadGraphError>> + 'a,
		errors::DownloadGraphError,
	>
	where
		Reporter: DownloadsReporter + Send + Sync + 'static,
	{
		let semaphore = Arc::new(Semaphore::new(network_concurrency.get()));

		let mut tasks = package_ids
			.map(|package_id| {
				let span = tracing::info_span!("download", package_id = package_id.to_string());

				let project = self.clone();
				let reporter = reporter.cloned();
				let refreshed_sources = refreshed_sources.clone();
				let semaphore = semaphore.clone();
				let source_state = lockfile.source_states[package_id.source()].clone();
				let package = lockfile.graph.resolved_package(package_id).unwrap();

				async move {
					let _permit = semaphore.acquire().await;

					let progress_reporter = reporter
						.clone()
						.map(|reporter| reporter.report_download(package.id.to_string()));

					if let Some(progress_reporter) = &progress_reporter {
						progress_reporter.report_start();
					}

					let source = package.id.source();
					refreshed_sources.refresh_index(source, &project).await?;

					tracing::debug!("downloading");

					let fs = match progress_reporter {
						Some(progress_reporter) => {
							source
								.download(
									&project,
									&source_state,
									&package,
									progress_reporter.into(),
								)
								.await
						}
						None => {
							source
								.download(&project, &source_state, &package, ().into())
								.await
						}
					}?;

					tracing::debug!("downloaded");

					Ok((package.id, fs))
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
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadGraphError))]
	#[non_exhaustive]
	pub enum DownloadGraphErrorKind {
		/// An error occurred refreshing a package source index
		#[error("failed to refresh package source index")]
		RefreshIndexFailed(#[from] crate::source::errors::RefreshIndexError),

		/// Error interacting with the filesystem
		#[error("error interacting with the filesystem")]
		Io(#[from] std::io::Error),

		/// Error downloading a package
		#[error("failed to download package")]
		DownloadFailed(#[from] crate::source::errors::DownloadError),
	}
}
