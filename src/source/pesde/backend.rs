//! pesde package source backend abstraction
#![allow(async_fn_in_trait)]

use crate::Project;
use crate::names::PackageName;
use crate::reporters::DownloadProgressReporter;
use futures::Stream;
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

/// All available pesde package backends
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PesdePackageBackends {}

impl Display for PesdePackageBackends {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		todo!()
	}
}

impl FromStr for PesdePackageBackends {
	type Err = errors::ParseBackendError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Err(errors::ParseBackendErrorKind::NoMatch(s.to_string(), "".into()).into())
	}
}

crate::ser_display_deser_fromstr!(PesdePackageBackends);

impl PesdePackageSourceBackend for PesdePackageBackends {
	type RefreshError = errors::RefreshError;
	type DownloadError = errors::DownloadError;

	async fn refresh(&self, _project: &Project) -> Result<(), Self::RefreshError> {
		todo!()
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
		todo!();
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
		#[error("no backend type matched for {0}")]
		NoMatch(
			String,
			#[source] /* TODO */ Box<dyn std::error::Error + Send + Sync>,
		),
	}

	/// Errors that can occur when refreshing a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = RefreshError))]
	#[non_exhaustive]
	pub enum RefreshErrorKind {
		// /// An error occurred from the Git backend
		// #[error("error from git backend")]
		// Git(#[from] crate::source::git_index::errors::RefreshError),
	}

	/// Errors that can occur when reading the config file for a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ConfigError))]
	#[non_exhaustive]
	pub enum ConfigErrorKind {
		// /// An error occurred from the Git backend
		// #[error("error from git backend")]
		// Git(#[from] GitConfigError),
	}

	/// Errors that can occur when reading an index file for a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ReadIndexFileError))]
	#[non_exhaustive]
	pub enum ReadIndexFileErrorKind {
		// /// An error occurred from the Git backend
		// #[error("error from git backend")]
		// Git(#[from] GitReadIndexFileError),
	}

	/// Errors that can occur when downloading a package from a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		// /// An error occurred from the Git backend
		// #[error("error from git backend")]
		// Git(#[from] GitDownloadError),
	}
}
