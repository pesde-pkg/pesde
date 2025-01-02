//! Progress reporting
//!
//! Certain operations will ask for a progress reporter to be passed in, this
//! allows the caller to be notified of progress during the operation. This can
//! be used to show progress to the user.
//!
//! All reporter traits are implemented for `()`. These implementations do
//! nothing, and can be used to ignore progress reporting.

#![allow(unused_variables)]

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
