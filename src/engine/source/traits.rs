use crate::{engine::source::archive::Archive, reporters::DownloadProgressReporter};
use semver::{Version, VersionReq};
use std::{collections::BTreeMap, fmt::Debug, future::Future, path::PathBuf, sync::Arc};
use tokio::io::AsyncBufRead;

/// Options for resolving an engine
#[derive(Debug, Clone)]
pub struct ResolveOptions {
	/// The reqwest client to use
	pub reqwest: reqwest::Client,
}

/// Options for downloading an engine
#[derive(Debug, Clone)]
pub struct DownloadOptions<R: DownloadProgressReporter> {
	/// The reqwest client to use
	pub reqwest: reqwest::Client,
	/// The reporter to use
	pub reporter: Arc<R>,
	/// The version of the engine to be downloaded
	pub version: Version,
}

/// A source of engines
pub trait EngineSource: Debug {
	/// The reference type for this source
	type Ref;
	/// The error type for resolving an engine from this source
	type ResolveError: std::error::Error + Send + Sync + 'static;
	/// The error type for downloading an engine from this source
	type DownloadError: std::error::Error + Send + Sync + 'static;

	/// Returns the folder to store the engine's versions in
	fn directory(&self) -> PathBuf;

	/// Returns the expected file name of the engine in the archive
	fn expected_file_name(&self) -> &str;

	/// Resolves a requirement to a reference
	fn resolve(
		&self,
		requirement: &VersionReq,
		options: &ResolveOptions,
	) -> impl Future<Output = Result<BTreeMap<Version, Self::Ref>, Self::ResolveError>> + Send + Sync;

	/// Downloads an engine
	fn download<R: DownloadProgressReporter + 'static>(
		&self,
		engine_ref: &Self::Ref,
		options: &DownloadOptions<R>,
	) -> impl Future<Output = Result<Archive<impl AsyncBufRead + 'static>, Self::DownloadError>>
	       + Send
	       + Sync;
}
