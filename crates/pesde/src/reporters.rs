//! Progress reporting
//!
//! Certain operations will ask for a progress reporter to be passed in, this
//! allows the caller to be notified of progress during the operation. This can
//! be used to show progress to the user.
//!
//! All reporter traits are implemented for `()`. These implementations do
//! nothing, and can be used to ignore progress reporting.

#![expect(unused_variables)]

use std::sync::Arc;

/// Reports downloads.
pub trait DownloadsReporter: Send + Sync {
	/// The [`DownloadProgressReporter`] type associated with this reporter.
	type DownloadProgressReporter: DownloadProgressReporter + 'static;

	/// Starts a new download.
	fn report_download(self: Arc<Self>, name: String) -> Self::DownloadProgressReporter;
}

impl DownloadsReporter for () {
	type DownloadProgressReporter = ();
	fn report_download(self: Arc<Self>, name: String) -> Self::DownloadProgressReporter {}
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
pub trait PatchesReporter: Send + Sync {
	/// The [`PatchProgressReporter`] type associated with this reporter.
	type PatchProgressReporter: PatchProgressReporter + 'static;

	/// Starts a new patch.
	fn report_patch(self: Arc<Self>, name: String) -> Self::PatchProgressReporter;
}

impl PatchesReporter for () {
	type PatchProgressReporter = ();
	fn report_patch(self: Arc<Self>, name: String) -> Self::PatchProgressReporter {}
}

/// Reports the progress of a single patch.
pub trait PatchProgressReporter: Send + Sync {
	/// Reports that the patch has been applied.
	fn report_done(&self) {}
}

impl PatchProgressReporter for () {}
