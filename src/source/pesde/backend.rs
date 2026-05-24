//! pesde package source backend abstraction
#![allow(async_fn_in_trait)]

use crate::Project;
use crate::names::PackageName;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
use futures::Stream;
use futures::TryStreamExt as _;
use relative_path::RelativePathBuf;
use semver::Version;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;
use std::sync::Arc;

/// A source of  pesde packages
pub trait PesdePackageSourceBackend: Debug + Display + Send + Sync {
	/// The error type for refreshing this backend
	type RefreshError: std::error::Error + Send + Sync + 'static;
	/// The error type for downloading entries
	type DownloadError: std::error::Error + Send + Sync + 'static;

	/// Refreshes the backend
	fn refresh(
		&self,
		project: &Project,
	) -> impl Future<Output = Result<(), Self::RefreshError>> + Send;

	/// Downloads entries for a package version
	fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
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
	api_url: Arc<url::Url>,
}
ser_display_deser_fromstr!(ApiPesdePackageSourceBackend);

impl Display for ApiPesdePackageSourceBackend {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.api_url)
	}
}

impl FromStr for ApiPesdePackageSourceBackend {
	type Err = url::ParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse::<url::Url>().map(Self::new)
	}
}

impl ApiPesdePackageSourceBackend {
	/// Creates a new API pesde package source backend
	#[must_use]
	pub fn new(api_url: impl Into<Arc<url::Url>>) -> Self {
		Self {
			api_url: api_url.into(),
		}
	}

	/// Gets the API URL
	#[must_use]
	pub fn api_url(&self) -> &url::Url {
		&self.api_url
	}
}

impl PesdePackageSourceBackend for ApiPesdePackageSourceBackend {
	type RefreshError = errors::ApiRefreshError;
	type DownloadError = errors::ApiDownloadError;

	async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
		Ok(())
	}

	async fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		package: &PackageName,
		version: &Version,
		reporter: Arc<R>,
	) -> Result<
		impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
		Self::DownloadError,
	> {
		Ok(futures::stream::empty())
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

	async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
		match self {
			Self::Api(repo) => repo.refresh(project).await.map_err(Into::into),
		}
	}

	async fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		package: &PackageName,
		version: &Version,
		reporter: Arc<R>,
	) -> Result<
		impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
		Self::DownloadError,
	> {
		Ok(match self {
			Self::Api(repo) => repo
				.download_entries(project, package, version, reporter)
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
		NoMatch(String, #[source] url::ParseError),
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
	pub enum ApiRefreshErrorKind {}

	/// Errors that can occur when downloading from an API pesde package source backend
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ApiDownloadError))]
	#[non_exhaustive]
	pub enum ApiDownloadErrorKind {}

	/// Errors that can occur when parsing a scope permission from a string
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ScopePermissionFromStrError))]
	pub enum ScopePermissionFromStrErrorKind {
		/// Unknown scope permission
		#[error("unknown scope permission `{0}`")]
		UnknownScopePermission(String),
	}
}
