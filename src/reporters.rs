//! Progress reporting
//!
//! Certain operations will ask for a progress reporter to be passed in, this
//! allows the caller to be notified of progress during the operation. This can
//! be used to show progress to the user.
//!
//! All reporter traits are implemented for `()`. These implementations do
//! nothing, and can be used to ignore progress reporting.

#![allow(unused_variables)]

use async_stream::stream;
use futures::StreamExt;
use std::sync::Arc;
use tokio::io::AsyncBufRead;

/// Reports downloads.
pub trait DownloadsReporter<'a>: Send + Sync {
	/// The [`DownloadProgressReporter`] type associated with this reporter.
	type DownloadProgressReporter: DownloadProgressReporter + 'a;

	/// Starts a new download.
	fn report_download<'b>(&'a self, name: &'b str) -> Self::DownloadProgressReporter;
}

impl DownloadsReporter<'_> for () {
	type DownloadProgressReporter = ();
	fn report_download(&self, name: &str) -> Self::DownloadProgressReporter {}
}

/// Reports the progress of a single download.
pub trait DownloadProgressReporter: Send + Sync {
	/// Reports that the download has started.
	fn report_start(&self) {}

	/// Reports the progress of the download.
	///
	/// `total` is the total number of bytes to download, and `len` is the number
	/// of bytes downloaded so far.
	fn report_progress(&self, total: u64, len: u64) {}

	/// Reports that the download is done.
	fn report_done(&self) {}
}

impl DownloadProgressReporter for () {}

/// Reports the progress of applying patches.
pub trait PatchesReporter<'a>: Send + Sync {
	/// The [`PatchProgressReporter`] type associated with this reporter.
	type PatchProgressReporter: PatchProgressReporter + 'a;

	/// Starts a new patch.
	fn report_patch<'b>(&'a self, name: &'b str) -> Self::PatchProgressReporter;
}

impl PatchesReporter<'_> for () {
	type PatchProgressReporter = ();
	fn report_patch(&self, name: &str) -> Self::PatchProgressReporter {}
}

/// Reports the progress of a single patch.
pub trait PatchProgressReporter: Send + Sync {
	/// Reports that the patch has been applied.
	fn report_done(&self) {}
}

impl PatchProgressReporter for () {}

pub(crate) fn response_to_async_read<R: DownloadProgressReporter>(
	response: reqwest::Response,
	reporter: Arc<R>,
) -> impl AsyncBufRead {
	let total_len = response.content_length().unwrap_or(0);
	reporter.report_progress(total_len, 0);

	let mut bytes_downloaded = 0;
	let mut stream = response.bytes_stream();
	let bytes = stream!({
		while let Some(chunk) = stream.next().await {
			let chunk = match chunk {
				Ok(chunk) => chunk,
				Err(err) => {
					yield Err(std::io::Error::new(std::io::ErrorKind::Other, err));
					continue;
				}
			};
			bytes_downloaded += chunk.len() as u64;
			reporter.report_progress(total_len, bytes_downloaded);
			yield Ok(chunk);
		}

		reporter.report_done();
	});

	tokio_util::io::StreamReader::new(bytes)
}
