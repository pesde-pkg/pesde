//! pesde package source backend abstraction
use crate::Project;
use crate::Url;
use crate::names::PackageName;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
use crate::source::pesde::PesdeSourceState;
use crate::source::pesde::registry::*;
use async_stream::try_stream;
use fs_err::tokio as fs;
use futures::Stream;
use futures::TryStreamExt as _;
use merkleberg::Merge as _;
use merkleberg::mmriver::InclusionProof;
use relative_path::RelativePathBuf;
use reqwest::RequestBuilder;
use reqwest::header::AUTHORIZATION;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;
use std::sync::Arc;
use tempfile::Builder;
use tokio::io::AsyncBufReadExt as _;
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncSeekExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::io::BufReader;
use tokio::task::spawn_blocking;

/// A source of pesde packages
pub trait PesdePackageSourceBackend: Debug + Display + Send + Sync {
	/// The error type for refreshing this backend
	type RefreshError: std::error::Error + Send + Sync + 'static;
	/// The error type for downloading entries
	type DownloadError: std::error::Error + Send + Sync + 'static;

	/// Refreshes the backend and fetches state
	fn refresh(
		&self,
		project: &Project,
		old_state: Option<&PesdeSourceState>,
	) -> impl Future<Output = Result<Option<LogHeadResponse>, Self::RefreshError>> + Send;

	/// Downloads entries for a package version
	fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		state: &PesdeSourceState,
		package: &PackageName,
		version: &Version,
		reporter: Arc<R>,
	) -> impl Future<
		Output = Result<
			impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
			Self::DownloadError,
		>,
	> + Send;
}

/// An API-based pesde package source backend
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ApiPesdePackageSourceBackend {
	api_url: Url,
}
ser_display_deser_fromstr!(ApiPesdePackageSourceBackend);

impl Display for ApiPesdePackageSourceBackend {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.api_url)
	}
}

impl FromStr for ApiPesdePackageSourceBackend {
	type Err = crate::errors::ParseUrlError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self::new)
	}
}

impl ApiPesdePackageSourceBackend {
	/// Creates a new API pesde package source backend
	#[must_use]
	pub fn new(api_url: Url) -> Self {
		Self { api_url }
	}

	/// Gets the API URL
	#[must_use]
	pub fn api_url(&self) -> &Url {
		&self.api_url
	}

	fn api_url_str(&self) -> &str {
		let str = self.api_url().as_url().as_str();
		str.strip_suffix('/').unwrap_or(str)
	}

	fn authed_request(&self, project: &Project, request: RequestBuilder) -> RequestBuilder {
		if let Some(token) = project.auth_config().tokens().get(&self.api_url) {
			tracing::debug!("using token for {}", self.api_url);
			return request.header(AUTHORIZATION, token);
		}

		request
	}

	async fn included_entry<
		P: Serialize + DeserializeOwned,
		E: Into<Entry<P>> + DeserializeOwned,
	>(
		&self,
		project: &Project,
		state: &PesdeSourceState,
		url: String,
	) -> Result<Entry<P>, errors::ApiIncludedEntryError> {
		let entry = self
			.authed_request(project, project.reqwest().get(url))
			.send()
			.await?
			.error_for_status()?
			.json::<E>()
			.await?
			.into();

		let inclusion_proof = self
			.authed_request(
				project,
				project.reqwest().get(format!(
					"{}/v2/log/inclusion/{}",
					self.api_url_str(),
					entry.pos
				)),
			)
			.send()
			.await?
			.error_for_status()?
			.json::<InclusionProofResponse>()
			.await?;

		let inclusion_proof =
			InclusionProof::<CurrentMmrMerge>::new(entry.pos, inclusion_proof.proof);
		let nodehash = CurrentMmrMerge::leaf_hash(&canonical_bytes(&entry.payload)).unwrap();
		if !inclusion_proof.verify(nodehash, &state.accumulator.peaks)? {
			return Err(errors::ApiIncludedEntryErrorKind::InvalidInclusionProof.into());
		}

		Ok(entry)
	}
}

impl PesdePackageSourceBackend for ApiPesdePackageSourceBackend {
	type RefreshError = errors::ApiRefreshError;
	type DownloadError = errors::ApiDownloadError;

	async fn refresh(
		&self,
		project: &Project,
		old_state: Option<&PesdeSourceState>,
	) -> Result<Option<LogHeadResponse>, Self::RefreshError> {
		let response = self
			.authed_request(
				project,
				project.reqwest().get({
					let query = if let Some(old_state) = old_state {
						format_args!("?size_from={}", old_state.mmr_size)
					} else {
						format_args!("")
					};

					format!("{}/v2/log/head{query}", self.api_url_str())
				}),
			)
			.send()
			.await?;

		match response.status() {
			reqwest::StatusCode::OK => Ok(Some(response.json().await?)),
			// no packages have yet been published
			reqwest::StatusCode::NOT_FOUND => Ok(None),
			_ => response
				.error_for_status()
				.map(|_| None)
				.map_err(Into::into),
		}
	}

	async fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		state: &PesdeSourceState,
		package: &PackageName,
		version: &Version,
		reporter: Arc<R>,
	) -> Result<
		impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
		Self::DownloadError,
	> {
		let url_scope = urlencoding::encode(package.scope().as_str());
		let url_name = urlencoding::encode(package.name().as_str());
		let url_version = version.to_string();
		let url_version = urlencoding::encode(&url_version);

		#[derive(Debug, Deserialize)]
		#[serde(transparent)]
		struct PublishedEntry(PackageVersionResponse);

		impl From<PublishedEntry> for Entry<PublishScopeEntry> {
			fn from(value: PublishedEntry) -> Self {
				value.0.publish
			}
		}

		let package = self
			.included_entry::<PublishScopeEntry, PublishedEntry>(
				project,
				state,
				format!(
					"{}/v2/package/{url_scope}/{url_name}/{url_version}",
					self.api_url_str()
				),
			)
			.await?;

		let response = self
			.authed_request(
				project,
				project.reqwest().get(format!(
					"{}/v2/package/{url_scope}/{url_name}/{url_version}/archive",
					self.api_url_str()
				)),
			)
			.send()
			.await?
			.error_for_status()?;

		let stream = try_stream!({
			let archive_bytes =
				crate::reporters::response_to_async_buf_read(response, reporter.clone());
			tokio::pin!(archive_bytes);

			// TODO: verify
			let package = package.payload.into_unsafe_body();
			let payload_hash = package.payload.archive_hash;
			let mut hasher = payload_hash.algorithm().hasher();

			let temp_path = spawn_blocking(move || Builder::new().make(|_| Ok(())))
				.await
				.unwrap()
				.map_err(errors::ApiDownloadErrorKind::OpenArchive)?
				.into_temp_path();
			let mut archive_file = fs::File::create(temp_path.to_path_buf())
				.await
				.map_err(errors::ApiDownloadErrorKind::WriteBytes)?;

			loop {
				let bytes = archive_bytes
					.fill_buf()
					.await
					.map_err(errors::ApiDownloadErrorKind::ReadBytes)?;
				let bytes_amt = bytes.len();
				if bytes_amt == 0 {
					break;
				}

				hasher.update(bytes);
				archive_file
					.write_all(bytes)
					.await
					.map_err(errors::ApiDownloadErrorKind::WriteBytes)?;

				archive_bytes.consume(bytes_amt);
			}

			if hasher.finalize().as_ref() != payload_hash.hash().as_bytes() {
				Err(errors::ApiDownloadErrorKind::ArchiveIntegrityVerificationFailed)?;
			}

			archive_file
				.rewind()
				.await
				.map_err(errors::ApiDownloadErrorKind::WriteBytes)?;
			let decoder =
				async_compression::tokio::bufread::ZstdDecoder::new(BufReader::new(archive_file));
			let mut archive = tokio_tar::Archive::new(decoder);
			let mut entries_stream = archive
				.entries()
				.map_err(errors::ApiDownloadErrorKind::OpenArchive)?;

			while let Some(mut entry) = entries_stream
				.try_next()
				.await
				.map_err(errors::ApiDownloadErrorKind::ReadEntry)?
			{
				let path = entry
					.path()
					.map_err(errors::ApiDownloadErrorKind::ReadEntry)?;
				let path_str = path
					.to_str()
					.ok_or_else(|| errors::ApiDownloadErrorKind::InvalidPath)?;
				let rel_path = RelativePathBuf::from_path(path_str)
					.map_err(|_e| errors::ApiDownloadErrorKind::InvalidPath)?;

				if entry.header().entry_type().is_dir() {
					yield (rel_path, None);
					continue;
				}

				let mut contents = Vec::new();
				entry
					.read_to_end(&mut contents)
					.await
					.map_err(errors::ApiDownloadErrorKind::ReadEntry)?;

				yield (rel_path, Some(contents));
			}
		});

		Ok(stream)
	}
}

/// All available pesde package backends
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PesdePackageBackends {
	/// An API-based pesde package source backend
	Api(ApiPesdePackageSourceBackend),
}
ser_display_deser_fromstr!(PesdePackageBackends);

impl Display for PesdePackageBackends {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Api(repo) => write!(f, "{repo}"),
		}
	}
}

impl FromStr for PesdePackageBackends {
	type Err = errors::ParseBackendError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let url_err = match s.parse() {
			Ok(repo) => return Ok(PesdePackageBackends::Api(repo)),
			Err(e) => e,
		};

		Err(errors::ParseBackendErrorKind::NoMatch(s.to_string(), url_err).into())
	}
}

impl PesdePackageSourceBackend for PesdePackageBackends {
	type RefreshError = errors::RefreshError;
	type DownloadError = errors::DownloadError;

	async fn refresh(
		&self,
		project: &Project,
		old_state: Option<&PesdeSourceState>,
	) -> Result<Option<LogHeadResponse>, Self::RefreshError> {
		match self {
			Self::Api(repo) => repo.refresh(project, old_state).await.map_err(Into::into),
		}
	}

	async fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		state: &PesdeSourceState,
		package: &PackageName,
		version: &Version,
		reporter: Arc<R>,
	) -> Result<
		impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
		Self::DownloadError,
	> {
		Ok(match self {
			Self::Api(repo) => repo
				.download_entries(project, state, package, version, reporter)
				.await?
				.map_err(Into::into),
		})
	}
}

/// Errors that can occur when interacting with pesde package source backends
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when parsing a pesde package source backend
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ParseBackendError))]
	pub enum ParseBackendErrorKind {
		/// No backend type matched the input
		#[error("no backend type matched for `{0}`")]
		NoMatch(String, #[source] crate::errors::ParseUrlError),
	}

	/// Errors that can occur when refreshing a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = RefreshError))]
	#[non_exhaustive]
	pub enum RefreshErrorKind {
		/// An error occurred from the API backend
		#[error("error from api backend")]
		Api(#[from] ApiRefreshError),
	}

	/// Errors that can occur when downloading a package from a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		/// An error occurred from the API backend
		#[error("error from api backend")]
		Api(#[from] ApiDownloadError),
	}

	/// Errors that can occur when refreshing an API pesde package source backend
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ApiRefreshError))]
	#[non_exhaustive]
	pub enum ApiRefreshErrorKind {
		/// An error occurred while interacting with reqwest
		#[error("error interacting with reqwest")]
		ReqwestError(#[from] reqwest::Error),
	}

	/// Errors that can occur when attempting to get and validate an entry in the API pesde package source backend
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ApiIncludedEntryError))]
	#[non_exhaustive]
	pub enum ApiIncludedEntryErrorKind {
		/// An error occurred while interacting with reqwest
		#[error("error interacting with reqwest")]
		ReqwestError(#[from] reqwest::Error),

		/// An error occurred while interacting with Merkleberg
		#[error("error interacting with merkleberg")]
		MerklebergError(#[from] merkleberg::Error),

		/// The inclusion proof verification has failed
		#[error("inclusion proof couldn't verify entry")]
		InvalidInclusionProof,
	}

	/// Errors that can occur when downloading from an API pesde package source backend
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ApiDownloadError))]
	#[non_exhaustive]
	pub enum ApiDownloadErrorKind {
		/// An error occurred while sending the request
		#[error("error sending request to API")]
		RequestError(#[from] reqwest::Error),

		/// An error occurred while attempting to fetch an included entry
		#[error("error fetching included entry")]
		IncludedEntry(#[from] ApiIncludedEntryError),

		/// An error occurred while reading the archive bytes
		#[error("error reading archive bytes")]
		ReadBytes(#[source] std::io::Error),

		/// An error occurred while writing the archive bytes
		#[error("error writing archive bytes")]
		WriteBytes(#[source] std::io::Error),

		/// The archive failed integrity verification
		#[error("error validating archive integrity")]
		ArchiveIntegrityVerificationFailed,

		/// An error occurred opening the archive
		#[error("error opening archive")]
		OpenArchive(#[source] std::io::Error),

		/// An error occurred reading an entry from the archive
		#[error("error reading entry from archive")]
		ReadEntry(#[source] std::io::Error),

		/// An invalid path was encountered in the archive
		#[error("invalid path in archive")]
		InvalidPath,
	}
}
