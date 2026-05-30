//! Wally package source backend abstraction
#![allow(async_fn_in_trait)]

use crate::GixUrl;
use crate::Project;
use crate::names::WallyPackageName;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
use crate::source::git_index::read_file;
use crate::source::git_index::root_tree;
use crate::util::ToEscaped as _;
use async_stream::try_stream;
use futures::AsyncReadExt as _;
use futures::Stream;
use futures::TryStreamExt as _;
use relative_path::RelativePathBuf;
use reqwest::header::AUTHORIZATION;
use semver::Version;
use serde::Deserialize;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::AsyncReadExt as _;
use tokio::io::BufReader;
use tokio::task::spawn_blocking;
use tracing::instrument;

/// A Wally index config
#[derive(Debug, Clone)]
pub struct WallyIndexConfig {
	/// The API URL for the Wally registry
	pub api: url::Url,
	/// Fallback registries to use if the primary registry is unavailable
	pub fallback_registries: Vec<WallyPackageBackends>,
}

/// A source of Wally packages
pub trait WallyPackageSourceBackend: Debug + Display + Send + Sync {
	/// The error type for refreshing this backend
	type RefreshError: std::error::Error + Send + Sync + 'static;
	/// The error type for reading config
	type ConfigError: std::error::Error + Send + Sync + 'static;
	/// The error type for reading index files
	type ReadIndexFileError: std::error::Error + Send + Sync + 'static;
	/// The error type for downloading entries
	type DownloadError: std::error::Error + Send + Sync + 'static;

	/// Refreshes the backend's index
	fn refresh(
		&self,
		project: &Project,
	) -> impl Future<Output = Result<(), Self::RefreshError>> + Send;

	/// Reads the config for this backend
	fn config(
		&self,
		project: &Project,
	) -> impl Future<Output = Result<WallyIndexConfig, Self::ConfigError>> + Send;

	/// Reads an index file for a package
	fn read_index_file(
		&self,
		project: &Project,
		pkg_name: WallyPackageName,
	) -> impl Future<Output = Result<Option<String>, Self::ReadIndexFileError>> + Send;

	/// Downloads entries for a package version
	fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		pkg_name: &WallyPackageName,
		version: &Version,
		reporter: Arc<R>,
	) -> impl Future<
		Output = Result<
			impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
			Self::DownloadError,
		>,
	> + Send;
}

/// A Git-based Wally package source backend
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GitWallyPackageSourceBackend {
	repo_url: GixUrl,
}
ser_display_deser_fromstr!(GitWallyPackageSourceBackend);

impl Display for GitWallyPackageSourceBackend {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.repo_url)
	}
}

impl FromStr for GitWallyPackageSourceBackend {
	type Err = crate::errors::GixUrlError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self::new)
	}
}

impl GitWallyPackageSourceBackend {
	/// Creates a new Git Wally package source backend
	#[must_use]
	pub fn new(repo_url: GixUrl) -> Self {
		Self { repo_url }
	}

	fn path(&self, project: &Project) -> PathBuf {
		project
			.data_dir()
			.join("git_repos")
			.join("wally")
			.join(self.repo_url.to_string().escaped())
	}

	/// Gets the repository URL
	#[must_use]
	pub fn repo_url(&self) -> &GixUrl {
		&self.repo_url
	}
}

impl WallyPackageSourceBackend for GitWallyPackageSourceBackend {
	type RefreshError = errors::GitRefreshError;
	type ConfigError = errors::GitConfigError;
	type ReadIndexFileError = errors::GitReadIndexFileError;
	type DownloadError = errors::GitDownloadError;

	#[instrument(skip_all, level = "debug")]
	async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
		crate::source::git_index::refresh_git_repo(self.path(project), self.repo_url.clone())
			.await
			.map_err(Into::into)
	}

	#[instrument(skip_all, ret(level = "trace"), level = "debug")]
	async fn config(&self, project: &Project) -> Result<WallyIndexConfig, Self::ConfigError> {
		let repo_url = self.repo_url.clone();
		let path = self.path(project);

		spawn_blocking(move || {
			let repo = gix::open(&path)?;
			let tree = root_tree(&repo)?;
			let file = read_file(&tree, ["config.json"])?;

			match file {
				Some(s) => {
					let git_config: GitWallyIndexConfig = serde_json::from_str(&s)?;
					Ok(WallyIndexConfig::from(git_config))
				}
				None => Err(errors::GitConfigErrorKind::Missing(repo_url).into()),
			}
		})
		.await
		.unwrap()
	}

	async fn read_index_file(
		&self,
		project: &Project,
		pkg_name: WallyPackageName,
	) -> Result<Option<String>, Self::ReadIndexFileError> {
		let path = self.path(project);

		spawn_blocking(move || {
			let repo: Result<gix::Repository, errors::GitReadIndexFileError> =
				gix::open(&path).map_err(|e| errors::GitReadIndexFileErrorKind::Open(e).into());
			let repo = repo?;
			let tree: Result<gix::Tree, errors::GitReadIndexFileError> =
				root_tree(&repo).map_err(|e| errors::GitReadIndexFileErrorKind::Tree(e).into());
			let tree = tree?;
			read_file(&tree, [pkg_name.scope(), pkg_name.name()])
				.map_err(|e| errors::GitReadIndexFileErrorKind::ReadFile(e).into())
		})
		.await
		.unwrap()
	}

	async fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		pkg_name: &WallyPackageName,
		version: &Version,
		reporter: Arc<R>,
	) -> Result<
		impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
		Self::DownloadError,
	> {
		let config = self.config(project).await?;

		let mut request = project
			.reqwest()
			.get(format!(
				"{}/v1/package-contents/{}/{}/{}",
				config.api.as_str().trim_end_matches('/'),
				urlencoding::encode(pkg_name.scope()),
				urlencoding::encode(pkg_name.name()),
				urlencoding::encode(&version.to_string())
			))
			.header(
				"Wally-Version",
				std::env::var("PESDE_WALLY_VERSION")
					.as_deref()
					.unwrap_or("0.3.2"),
			);

		if let Some(token) = project.auth_config().tokens().get(&self.repo_url) {
			tracing::debug!("using token for {}", self.repo_url);
			request = request.header(AUTHORIZATION, token);
		}

		let response = request.send().await?.error_for_status()?;

		let total_len = response.content_length().unwrap_or(0);
		let bytes = crate::reporters::response_to_async_buf_read(response, reporter.clone());
		tokio::pin!(bytes);

		let mut archive_bytes = Vec::with_capacity(total_len as usize);
		bytes
			.read_to_end(&mut archive_bytes)
			.await
			.map_err(errors::GitDownloadErrorKind::ReadEntryContents)?;

		let stream = try_stream!({
			let zip_file = BufReader::new(std::io::Cursor::new(archive_bytes));

			let mut archive =
				async_zip::tokio::read::seek::ZipFileReader::with_tokio(zip_file).await?;

			for index in 0..archive.file().entries().len() {
				let entry = archive.file().entries().get(index).unwrap();
				let entry_name = entry.filename().as_str()?;

				let path = RelativePathBuf::from_path(entry_name)?;

				let is_dir = entry.dir()?;

				if is_dir {
					yield (path, None);
					continue;
				}

				let mut entry_reader = archive.reader_without_entry(index).await?;

				let mut contents = Vec::new();
				entry_reader
					.read_to_end(&mut contents)
					.await
					.map_err(errors::GitDownloadErrorKind::ReadEntryContents)?;

				yield (path, Some(contents));
			}
		});

		Ok(stream)
	}
}

/// All available Wally package backends
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum WallyPackageBackends {
	/// A Git-based Wally package source backend
	Git(GitWallyPackageSourceBackend),
}
ser_display_deser_fromstr!(WallyPackageBackends);

impl Display for WallyPackageBackends {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Git(repo) => write!(f, "{repo}"),
		}
	}
}

impl FromStr for WallyPackageBackends {
	type Err = errors::ParseBackendError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let git_err = match s.parse::<GitWallyPackageSourceBackend>() {
			Ok(repo) => return Ok(Self::Git(repo)),
			Err(e) => e,
		};

		Err(errors::ParseBackendErrorKind::NoMatch(s.to_string(), git_err).into())
	}
}

impl WallyPackageSourceBackend for WallyPackageBackends {
	type RefreshError = errors::RefreshError;
	type ConfigError = errors::ConfigError;
	type ReadIndexFileError = errors::ReadIndexFileError;
	type DownloadError = errors::DownloadError;

	async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
		match self {
			Self::Git(repo) => repo.refresh(project).await.map_err(Into::into),
		}
	}

	async fn config(&self, project: &Project) -> Result<WallyIndexConfig, Self::ConfigError> {
		match self {
			Self::Git(repo) => repo.config(project).await.map_err(Into::into),
		}
	}

	async fn read_index_file(
		&self,
		project: &Project,
		pkg_name: WallyPackageName,
	) -> Result<Option<String>, Self::ReadIndexFileError> {
		match self {
			Self::Git(repo) => repo
				.read_index_file(project, pkg_name)
				.await
				.map_err(Into::into),
		}
	}

	async fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		pkg_name: &WallyPackageName,
		version: &Version,
		reporter: Arc<R>,
	) -> Result<
		impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
		Self::DownloadError,
	> {
		Ok(match self {
			Self::Git(repo) => repo
				.download_entries(project, pkg_name, version, reporter)
				.await?
				.map_err(Into::into),
		})
	}
}

#[derive(Debug, Clone, Deserialize)]
struct GitWallyIndexConfig {
	api: url::Url,
	#[serde(default)]
	fallback_registries: Vec<String>,
}

impl From<GitWallyIndexConfig> for WallyIndexConfig {
	fn from(git_config: GitWallyIndexConfig) -> Self {
		Self {
			api: git_config.api,
			fallback_registries: git_config
				.fallback_registries
				.into_iter()
				.filter_map(|url| {
					url.parse()
						.map(|url| {
							WallyPackageBackends::Git(GitWallyPackageSourceBackend::new(url))
						})
						// ignore instead of erroring so that a misconfigured fallback registry doesn't prevent the index from working entirely
						.map_err(|e| tracing::warn!("invalid fallback registry URL: {e}"))
						.ok()
				})
				.collect(),
		}
	}
}

/// Errors that can occur when interacting with Wally package source backends
pub mod errors {
	use crate::GixUrl;
	use crate::source::git_index::errors::ReadFile;
	use crate::source::git_index::errors::TreeError;
	use thiserror::Error;

	/// Errors that can occur when parsing a Wally package source backend
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ParseBackendError))]
	pub enum ParseBackendErrorKind {
		/// No backend type matched the input
		#[error("no backend type matched for `{0}`")]
		NoMatch(String, #[source] crate::errors::GixUrlError),
	}

	/// Errors that can occur when refreshing a Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = RefreshError))]
	#[non_exhaustive]
	pub enum RefreshErrorKind {
		/// An error occurred from the Git backend
		#[error("error from git backend")]
		Git(#[from] GitRefreshError),
	}

	/// Errors that can occur when refreshing a Git-based Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GitRefreshError))]
	#[non_exhaustive]
	pub enum GitRefreshErrorKind {
		/// An error occurred refreshing the git repository
		#[error("error refreshing git repository")]
		Refresh(#[from] crate::source::git_index::errors::RefreshIndexError),
	}

	/// Errors that can occur when reading the config file for a Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ConfigError))]
	#[non_exhaustive]
	pub enum ConfigErrorKind {
		/// An error occurred from the Git backend
		#[error("error from git backend")]
		Git(#[from] GitConfigError),
	}

	/// Errors that can occur when reading an index file for a Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ReadIndexFileError))]
	#[non_exhaustive]
	pub enum ReadIndexFileErrorKind {
		/// An error occurred from the Git backend
		#[error("error from git backend")]
		Git(#[from] GitReadIndexFileError),
	}

	/// Errors that can occur when downloading a package from a Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		/// An error occurred from the Git backend
		#[error("error from git backend")]
		Git(#[from] GitDownloadError),
	}

	/// Errors that can occur when reading the config file from a Git-based Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GitConfigError))]
	#[non_exhaustive]
	pub enum GitConfigErrorKind {
		/// An error occurred opening the repository
		#[error("error opening repository")]
		Open(#[from] gix::open::Error),

		/// An error occurred getting the tree
		#[error("error getting tree")]
		Tree(#[from] TreeError),

		/// An error occurred reading the config file
		#[error("error reading config file")]
		ReadFile(#[from] ReadFile),

		/// An error occurred parsing the config file
		#[error("error parsing config file")]
		Parse(#[from] serde_json::Error),

		/// The config file was missing for the index
		#[error("missing config file for index at {0}")]
		Missing(GixUrl),
	}

	/// Errors that can occur when reading an index file from a Git-based Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GitReadIndexFileError))]
	#[non_exhaustive]
	pub enum GitReadIndexFileErrorKind {
		/// An error occurred opening the repository
		#[error("error opening repository")]
		Open(#[from] gix::open::Error),

		/// An error occurred getting the tree
		#[error("error getting tree")]
		Tree(#[from] TreeError),

		/// An error occurred reading the file
		#[error("error reading file")]
		ReadFile(#[from] ReadFile),
	}

	/// Errors that can occur when downloading a package from a Git-based Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GitDownloadError))]
	#[non_exhaustive]
	pub enum GitDownloadErrorKind {
		/// An error occurred reading the config
		#[error("error reading config")]
		Config(#[from] GitConfigError),

		/// An error occurred downloading the package
		#[error("error downloading package")]
		Download(#[from] reqwest::Error),

		/// An error occurred interacting with async-zip
		#[error("error interacting with zip archive")]
		ZipArchive(#[from] async_zip::error::ZipError),

		/// An error occurred reading entry contents from the archive
		#[error("error reading entry contents from archive")]
		ReadEntryContents(#[source] std::io::Error),

		/// An error occurred parsing an entry path
		#[error("error parsing entry path")]
		InvalidPath(#[from] relative_path::FromPathError),
	}
}
