//! pesde package source backend abstraction
use crate::Project;
use crate::Url;
use crate::names::PackageName;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
use crate::source::pesde::PesdeSourceState;
use crate::source::pesde::registry::LogHeadResponse;
use async_stream::try_stream;
use futures::Stream;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use relative_path::RelativePathBuf;
use reqwest::header::AUTHORIZATION;
use semver::Version;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::AsyncReadExt as _;

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
}

impl PesdePackageSourceBackend for ApiPesdePackageSourceBackend {
	type RefreshError = errors::ApiRefreshError;
	type DownloadError = errors::ApiDownloadError;

	async fn refresh(
		&self,
		project: &Project,
		old_state: Option<&PesdeSourceState>,
	) -> Result<Option<LogHeadResponse>, Self::RefreshError> {
		let mut url = format!(
			"{}/v2/log/head",
			self.api_url().as_url().as_str().trim_end_matches('/')
		)
		.parse::<url::Url>()?;
		if let Some(old_state) = old_state {
			url.query_pairs_mut()
				.append_pair("size_from", &old_state.mmr_size.to_string());
		}

		let mut request = project.reqwest().get(url);
		if let Some(token) = project.auth_config().tokens().get(&self.api_url) {
			tracing::debug!("using token for {}", self.api_url);
			request = request.header(AUTHORIZATION, token);
		}

		let response = request.send().await?;
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
		_state: &PesdeSourceState,
		package: &PackageName,
		version: &Version,
		reporter: Arc<R>,
	) -> Result<
		impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
		Self::DownloadError,
	> {
		let url = format!(
			"{}/v2/package/{}/{}/{}/archive",
			self.api_url().as_url().as_str().trim_end_matches('/'),
			urlencoding::encode(package.scope().as_str()),
			urlencoding::encode(package.name().as_str()),
			urlencoding::encode(&version.to_string()),
		)
		.parse::<url::Url>()?;

		let mut request = project.reqwest().get(url);
		if let Some(token) = project.auth_config().tokens().get(&self.api_url) {
			tracing::debug!("using token for {}", self.api_url);
			request = request.header(AUTHORIZATION, token);
		}
		let response = request.send().await?.error_for_status()?;

		// TODO: validate archive hash, package entry

		let stream = try_stream!({
			let bytes = crate::reporters::response_to_async_buf_read(response, reporter.clone());
			tokio::pin!(bytes);

			let decoder = async_compression::tokio::bufread::GzipDecoder::new(bytes);
			let archive = async_tar::Archive::new(decoder);
			let mut entries_stream = archive
				.entries()
				.map_err(errors::ApiDownloadErrorKind::OpenArchive)?;

			while let Some(entry_result) = entries_stream.next().await {
				let mut entry = entry_result.map_err(errors::ApiDownloadErrorKind::ReadEntry)?;

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
		/// The built API url was invalid
		#[error("invalid API URL")]
		InvalidApiUrl(#[from] url::ParseError),

		/// An error occurred while sending the request
		#[error("error sending request to API")]
		RequestError(#[from] reqwest::Error),
	}

	/// Errors that can occur when downloading from an API pesde package source backend
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ApiDownloadError))]
	#[non_exhaustive]
	pub enum ApiDownloadErrorKind {
		/// The built API url was invalid
		#[error("invalid API URL")]
		InvalidApiUrl(#[from] url::ParseError),

		/// An error occurred while sending the request
		#[error("error sending request to API")]
		RequestError(#[from] reqwest::Error),

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
