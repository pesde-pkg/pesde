//! Legacy pesde package source backend abstraction
use crate::Project;
use crate::Url;
use crate::manifest::Alias;
use crate::manifest::DependencyType;
use crate::names::PackageName;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
use crate::source::git::specifier::IndexGitDependencySpecifier;
use crate::source::git_index::read_file;
use crate::source::git_index::root_tree;
use crate::source::legacy_pesde::specifier::IndexLegacyPesdeDependencySpecifier;
use crate::source::legacy_pesde::target::Target;
use crate::source::legacy_pesde::target::TargetKind;
use crate::source::wally::specifier::IndexWallyDependencySpecifier;
use crate::util::ToEscaped as _;
use async_stream::try_stream;
use futures::Stream;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use relative_path::RelativePathBuf;
use reqwest::header::ACCEPT;
use reqwest::header::AUTHORIZATION;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::AsyncReadExt as _;
use tokio::task::spawn_blocking;
use tracing::instrument;
use urlencoding::encode;

fn default_archive_size() -> usize {
	4 * 1024 * 1024
}

/// The configuration for the pesde index
#[derive(Deserialize, Debug, Clone)]
pub struct IndexConfig {
	/// The URL of the API
	pub api: Url,
	/// The URL to download packages from
	pub download: Option<String>,
	/// The OAuth client ID for GitHub
	#[serde(default)]
	pub github_oauth_client_id: Option<String>,
	/// The maximum size of an archive in bytes
	#[serde(default = "default_archive_size")]
	pub max_archive_size: usize,
	/// The packages to display in the CLI for default script implementations
	#[serde(default)]
	pub scripts_packages: Vec<PackageName>,
}

impl IndexConfig {
	/// The URL of the API
	#[must_use]
	pub fn api(&self) -> &str {
		self.api.as_url().as_str().trim_end_matches('/')
	}

	/// The URL to download packages from
	#[must_use]
	pub fn download(&self) -> String {
		self.download
			.as_deref()
			.unwrap_or("{API_URL}/v1/packages/{PACKAGE}/{PACKAGE_VERSION}/{PACKAGE_TARGET}/archive")
			.replace("{API_URL}", self.api())
	}
}

/// An entry in a package's documentation
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum DocEntryKind {
	/// A page in the documentation
	Page {
		/// The name of the page
		name: String,
		/// The hash of the page's content
		hash: String,
	},
	/// A category in the documentation
	Category {
		/// The items in the section
		#[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
		items: BTreeSet<DocEntry>,
		/// Whether this category is collapsed by default
		#[serde(default, skip_serializing_if = "std::ops::Not::not")]
		collapsed: bool,
	},
}

/// An entry in a package's documentation
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DocEntry {
	/// The label for this entry
	pub label: String,
	/// The position of this entry
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub position: Option<usize>,
	/// The kind of this entry
	#[serde(flatten)]
	pub kind: DocEntryKind,
}

impl Ord for DocEntry {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		match (self.position, other.position) {
			(Some(l), Some(r)) => l.cmp(&r),
			(Some(_), None) => std::cmp::Ordering::Less,
			(None, Some(_)) => std::cmp::Ordering::Greater,
			(None, None) => std::cmp::Ordering::Equal,
		}
		.then(self.label.cmp(&other.label))
	}
}

impl PartialOrd for DocEntry {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

/// The entry in a package's index file
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct IndexFileEntry {
	/// The target for this package
	pub target: Target,
	/// When this package was published
	pub published_at: jiff::Timestamp,
	/// The description of this package
	#[serde(default)]
	pub description: Option<String>,
	/// The license of this package
	#[serde(default)]
	pub license: Option<String>,
	/// The authors of this package
	#[serde(default)]
	pub authors: Vec<String>,
	/// The repository of this package
	#[serde(default)]
	pub repository: Option<Url>,
	/// The documentation for this package
	#[serde(default)]
	pub docs: BTreeSet<DocEntry>,
	/// Whether this version is yanked
	#[serde(default)]
	pub yanked: bool,
	/// The dependencies of this package
	#[serde(default)]
	pub dependencies: BTreeMap<Alias, (IndexDependencySpecifiers, DependencyType)>,
}

/// The dependency specifiers for an index file entry
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum IndexDependencySpecifiers {
	/// A legacy pesde dependency specifier
	LegacyPesde(IndexLegacyPesdeDependencySpecifier),
	/// A Wally dependency specifier
	Wally(IndexWallyDependencySpecifier),
	/// A Git dependency specifier
	Git(IndexGitDependencySpecifier),
}

/// The package metadata in the index file
#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexMetadata {
	/// Whether this package is deprecated
	#[serde(default)]
	pub deprecated: String,
}

/// The index file for a package
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct IndexFile {
	/// Any package-wide metadata
	#[serde(default)]
	pub meta: IndexMetadata,
	/// The entries in the index file
	#[serde(flatten)]
	pub entries: BTreeMap<VersionId, IndexFileEntry>,
}

/// A version ID, which is a combination of a version and a target
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VersionId(Version, TargetKind);
ser_display_deser_fromstr!(VersionId);

impl Display for VersionId {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}:{}", self.version(), self.target())
	}
}

impl FromStr for VersionId {
	type Err = errors::VersionIdParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (version, target) = s
			.split_once([':', ' '])
			.ok_or(errors::VersionIdParseErrorKind::Malformed(s.to_string()))?;

		let version = version.parse()?;
		let target = target.parse()?;

		Ok(VersionId(version, target))
	}
}

impl VersionId {
	/// Creates a new version ID
	#[must_use]
	pub fn new(version: Version, target: TargetKind) -> Self {
		VersionId(version, target)
	}

	/// Access the version
	#[must_use]
	pub fn version(&self) -> &Version {
		&self.0
	}

	/// Access the target
	#[must_use]
	pub fn target(&self) -> TargetKind {
		self.1
	}

	/// Returns this version ID as a string that can be used in the filesystem
	#[must_use]
	pub fn escaped(&self) -> String {
		format!("{}+{}", self.version(), self.target())
	}

	/// Access the parts of the version ID
	#[must_use]
	pub fn parts(&self) -> (&Version, TargetKind) {
		(self.version(), self.target())
	}

	/// Converts this version ID into its version component
	#[must_use]
	pub fn into_version(self) -> Version {
		self.0
	}
}

/// A source of legacy pesde packages
pub trait LegacyPesdePackageSourceBackend: Debug + Display + Send + Sync {
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
	) -> impl Future<Output = Result<IndexConfig, Self::ConfigError>> + Send;

	/// Reads an index file for a package
	fn read_index_file(
		&self,
		project: &Project,
		name: PackageName,
	) -> impl Future<Output = Result<Option<IndexFile>, Self::ReadIndexFileError>> + Send;

	/// Downloads entries for a package version
	fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		package: &PackageName,
		version_id: &VersionId,
		reporter: Arc<R>,
	) -> impl Future<
		Output = Result<
			impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
			Self::DownloadError,
		>,
	> + Send;
}

/// A Git-based legacy pesde package source backend
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GitLegacyPesdePackageSourceBackend {
	repo_url: Url,
}
ser_display_deser_fromstr!(GitLegacyPesdePackageSourceBackend);

impl Display for GitLegacyPesdePackageSourceBackend {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.repo_url)
	}
}

impl FromStr for GitLegacyPesdePackageSourceBackend {
	type Err = crate::errors::ParseUrlError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self::new)
	}
}

impl GitLegacyPesdePackageSourceBackend {
	/// Creates a new Git legacy pesde package source backend
	#[must_use]
	pub fn new(repo_url: Url) -> Self {
		Self { repo_url }
	}

	fn path(&self, project: &Project) -> PathBuf {
		project
			.data_dir()
			.join("git_repos")
			.join("pesde")
			.join(self.repo_url.to_string().escaped())
	}
}

impl LegacyPesdePackageSourceBackend for GitLegacyPesdePackageSourceBackend {
	type RefreshError = crate::source::git_index::errors::RefreshIndexError;
	type ConfigError = errors::GitConfigError;
	type ReadIndexFileError = errors::GitReadIndexFileError;
	type DownloadError = errors::GitDownloadError;

	#[instrument(skip_all, level = "debug")]
	async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
		crate::source::git_index::refresh_git_repo(self.path(project), self.repo_url.clone()).await
	}

	#[instrument(skip_all, ret(level = "trace"), level = "debug")]
	async fn config(&self, project: &Project) -> Result<IndexConfig, Self::ConfigError> {
		let repo_url = self.repo_url.clone();
		let path = self.path(project);

		spawn_blocking(move || {
			let repo = gix::open(&path)?;
			let tree = root_tree(&repo)?;
			let file = read_file(&tree, ["config.toml"])?;

			match file {
				Some(s) => toml::from_str(&s).map_err(Into::into),
				None => Err(errors::GitConfigErrorKind::Missing(repo_url).into()),
			}
		})
		.await
		.unwrap()
	}

	async fn read_index_file(
		&self,
		project: &Project,
		name: PackageName,
	) -> Result<Option<IndexFile>, Self::ReadIndexFileError> {
		let path = self.path(project);

		spawn_blocking(move || {
			let repo = gix::open(&path)?;
			let tree = root_tree(&repo)?;
			let string = match read_file(&tree, [name.scope().as_str(), name.name().as_str()]) {
				Ok(Some(s)) => s,
				Ok(None) => return Ok(None),
				Err(e) => {
					return Err(errors::GitReadIndexFileErrorKind::ReadFile(e).into());
				}
			};

			toml::from_str(&string).map_err(Into::into)
		})
		.await
		.unwrap()
	}

	async fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		package: &PackageName,
		version_id: &VersionId,
		reporter: Arc<R>,
	) -> Result<
		impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
		Self::DownloadError,
	> {
		let config = self.config(project).await?;
		let url = config
			.download()
			.replace("{PACKAGE}", &encode(&package.to_string()))
			.replace("{PACKAGE_VERSION}", &encode(&version_id.0.to_string()))
			.replace("{PACKAGE_TARGET}", &encode(&version_id.1.to_string()));

		let mut request = project
			.reqwest()
			.get(&url)
			.header(ACCEPT, "application/octet-stream");

		if let Some(token) = project.auth_config().tokens().get(&self.repo_url) {
			tracing::debug!("using token for {}", self.repo_url);
			request = request.header(AUTHORIZATION, token);
		}

		let response = request.send().await?.error_for_status()?;

		let stream = try_stream!({
			let bytes = crate::reporters::response_to_async_buf_read(response, reporter.clone());
			tokio::pin!(bytes);

			let decoder = async_compression::tokio::bufread::GzipDecoder::new(bytes);
			let archive = async_tar::Archive::new(decoder);
			let mut entries_stream = archive
				.entries()
				.map_err(errors::GitDownloadErrorKind::OpenArchive)?;

			while let Some(entry_result) = entries_stream.next().await {
				let mut entry = entry_result.map_err(errors::GitDownloadErrorKind::ReadEntry)?;

				let path = entry
					.path()
					.map_err(errors::GitDownloadErrorKind::ReadEntry)?;
				let path_str = path
					.to_str()
					.ok_or_else(|| errors::GitDownloadErrorKind::InvalidPath)?;
				let rel_path = RelativePathBuf::from_path(path_str)
					.map_err(|_e| errors::GitDownloadErrorKind::InvalidPath)?;

				if entry.header().entry_type().is_dir() {
					yield (rel_path, None);
					continue;
				}

				let mut contents = Vec::new();
				entry
					.read_to_end(&mut contents)
					.await
					.map_err(errors::GitDownloadErrorKind::ReadEntry)?;

				yield (rel_path, Some(contents));
			}
		});

		Ok(stream)
	}
}

/// All available pesde package backends
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum LegacyPesdePackageBackends {
	/// A Git-based legacy pesde package source backend
	Git(GitLegacyPesdePackageSourceBackend),
}
ser_display_deser_fromstr!(LegacyPesdePackageBackends);

impl Display for LegacyPesdePackageBackends {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Git(repo) => write!(f, "{repo}"),
		}
	}
}

impl FromStr for LegacyPesdePackageBackends {
	type Err = errors::ParseBackendError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let git_err = match s.parse::<GitLegacyPesdePackageSourceBackend>() {
			Ok(repo) => return Ok(Self::Git(repo)),
			Err(e) => e,
		};

		Err(errors::ParseBackendErrorKind::NoMatch(s.to_string(), git_err).into())
	}
}

impl LegacyPesdePackageSourceBackend for LegacyPesdePackageBackends {
	type RefreshError = errors::RefreshError;
	type ConfigError = errors::ConfigError;
	type ReadIndexFileError = errors::ReadIndexFileError;
	type DownloadError = errors::DownloadError;

	async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
		match self {
			Self::Git(repo) => repo.refresh(project).await.map_err(Into::into),
		}
	}

	async fn config(&self, project: &Project) -> Result<IndexConfig, Self::ConfigError> {
		match self {
			Self::Git(repo) => repo.config(project).await.map_err(Into::into),
		}
	}

	async fn read_index_file(
		&self,
		project: &Project,
		name: PackageName,
	) -> Result<Option<IndexFile>, Self::ReadIndexFileError> {
		match self {
			Self::Git(repo) => repo
				.read_index_file(project, name)
				.await
				.map_err(Into::into),
		}
	}

	async fn download_entries<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		package: &PackageName,
		version_id: &VersionId,
		reporter: Arc<R>,
	) -> Result<
		impl Stream<Item = Result<(RelativePathBuf, Option<Vec<u8>>), Self::DownloadError>> + Send,
		Self::DownloadError,
	> {
		Ok(match self {
			Self::Git(repo) => repo
				.download_entries(project, package, version_id, reporter)
				.await?
				.map_err(Into::into),
		})
	}
}

/// Errors that can occur when interacting with legacy pesde package source backends
pub mod errors {
	use crate::Url;
	use crate::source::git_index::errors::ReadFile;
	use crate::source::git_index::errors::TreeError;
	use crate::source::legacy_pesde::target::errors::TargetKindFromStr;
	use thiserror::Error;

	/// Errors that can occur when parsing a legacy pesde package source backend
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ParseBackendError))]
	pub enum ParseBackendErrorKind {
		/// No backend type matched the input
		#[error("no backend type matched for `{0}`")]
		NoMatch(String, #[source] crate::errors::ParseUrlError),
	}

	/// Errors that can occur when refreshing a legacy pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = RefreshError))]
	#[non_exhaustive]
	pub enum RefreshErrorKind {
		/// An error occurred from the Git backend
		#[error("error from git backend")]
		Git(#[from] crate::source::git_index::errors::RefreshIndexError),
	}

	/// Errors that can occur when reading the config file for a legacy pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ConfigError))]
	#[non_exhaustive]
	pub enum ConfigErrorKind {
		/// An error occurred from the Git backend
		#[error("error from git backend")]
		Git(#[from] GitConfigError),
	}

	/// Errors that can occur when reading an index file for a legacy pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ReadIndexFileError))]
	#[non_exhaustive]
	pub enum ReadIndexFileErrorKind {
		/// An error occurred from the Git backend
		#[error("error from git backend")]
		Git(#[from] GitReadIndexFileError),
	}

	/// Errors that can occur when downloading a package from a legacy pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		/// An error occurred from the Git backend
		#[error("error from git backend")]
		Git(#[from] GitDownloadError),
	}

	/// Errors that can occur when downloading a package from a Git-based legacy pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GitDownloadError))]
	#[non_exhaustive]
	pub enum GitDownloadErrorKind {
		/// An error occurred reading the config
		#[error("error reading config")]
		Config(#[from] GitConfigError),

		/// An error occurred downloading the archive
		#[error("error downloading archive")]
		Download(#[from] reqwest::Error),

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

	/// Errors that can occur when reading the config file from a Git-based legacy pesde package source
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
		Parse(#[from] toml::de::Error),

		/// The config file was missing for the index
		#[error("missing config file for index at {0}")]
		Missing(Url),
	}

	/// Errors that can occur when reading an index file from a Git-based legacy pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GitReadIndexFileError))]
	#[non_exhaustive]
	pub enum GitReadIndexFileErrorKind {
		/// An error occurred reading the file
		#[error("error reading file")]
		ReadFile(#[from] ReadFile),

		/// An error occurred opening the repository
		#[error("error opening repository")]
		Open(#[from] gix::open::Error),

		/// An error occurred getting the tree
		#[error("error getting tree")]
		Tree(#[from] TreeError),

		/// An error occurred parsing the file
		#[error("error parsing file")]
		Parse(#[from] toml::de::Error),
	}

	/// Errors that can occur when parsing a version ID
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = VersionIdParseError))]
	pub enum VersionIdParseErrorKind {
		/// The version ID was malformed
		#[error("malformed version ID `{0}`")]
		Malformed(String),

		/// An error occurred parsing the version
		#[error("error parsing version")]
		Version(#[from] semver::Error),

		/// An error occurred parsing the target
		#[error("error parsing target")]
		Target(#[from] TargetKindFromStr),
	}
}
