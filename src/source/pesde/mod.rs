use gix::Url;
use relative_path::RelativePathBuf;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use std::{
	collections::{BTreeMap, BTreeSet, HashSet},
	fmt::Debug,
	hash::Hash,
	path::PathBuf,
};
use tokio_util::io::StreamReader;

use pkg_ref::PesdePackageRef;
use specifier::PesdeDependencySpecifier;

use crate::{
	manifest::{target::Target, DependencyType},
	names::{PackageName, PackageNames},
	reporters::DownloadProgressReporter,
	source::{
		fs::{store_in_cas, FsEntry, PackageFs},
		git_index::{read_file, root_tree, GitBasedSource},
		traits::{DownloadOptions, GetTargetOptions, RefreshOptions, ResolveOptions},
		DependencySpecifiers, PackageSource, ResolveResult, VersionId, IGNORED_DIRS, IGNORED_FILES,
	},
	util::hash,
	Project,
};
use fs_err::tokio as fs;
use futures::StreamExt;
use tokio::task::spawn_blocking;
use tracing::instrument;

/// The pesde package reference
pub mod pkg_ref;
/// The pesde dependency specifier
pub mod specifier;

/// The pesde package source
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct PesdePackageSource {
	repo_url: Url,
}

/// The file containing scope information
pub const SCOPE_INFO_FILE: &str = "scope.toml";

/// Information about a scope
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeInfo {
	/// The people authorized to publish packages to this scope
	pub owners: BTreeSet<u64>,
}

impl GitBasedSource for PesdePackageSource {
	fn path(&self, project: &Project) -> PathBuf {
		project
			.data_dir()
			.join("indices")
			.join(hash(self.as_bytes()))
	}

	fn repo_url(&self) -> &Url {
		&self.repo_url
	}
}

impl PesdePackageSource {
	/// Creates a new pesde package source
	pub fn new(repo_url: Url) -> Self {
		Self { repo_url }
	}

	fn as_bytes(&self) -> Vec<u8> {
		self.repo_url.to_bstring().to_vec()
	}

	/// Reads the config file
	#[instrument(skip_all, ret(level = "trace"), level = "debug")]
	pub async fn config(&self, project: &Project) -> Result<IndexConfig, errors::ConfigError> {
		let repo_url = self.repo_url.clone();
		let path = self.path(project);

		spawn_blocking(move || {
			let repo = gix::open(&path).map_err(Box::new)?;
			let tree = root_tree(&repo).map_err(Box::new)?;
			let file = read_file(&tree, ["config.toml"]).map_err(Box::new)?;

			match file {
				Some(s) => toml::from_str(&s).map_err(Into::into),
				None => Err(errors::ConfigError::Missing(Box::new(repo_url))),
			}
		})
		.await
		.unwrap()
	}

	fn read_index_file(
		&self,
		name: &PackageName,
		project: &Project,
	) -> Result<Option<IndexFile>, errors::ReadIndexFileError> {
		let (scope, name) = name.as_str();
		let repo = gix::open(self.path(project)).map_err(Box::new)?;
		let tree = root_tree(&repo).map_err(Box::new)?;
		let string = match read_file(&tree, [scope, name]) {
			Ok(Some(s)) => s,
			Ok(None) => return Ok(None),
			Err(e) => {
				return Err(errors::ReadIndexFileError::ReadFile(e));
			}
		};

		toml::from_str(&string).map_err(Into::into)
	}
}

impl PackageSource for PesdePackageSource {
	type Specifier = PesdeDependencySpecifier;
	type Ref = PesdePackageRef;
	type RefreshError = crate::source::git_index::errors::RefreshError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetTargetError = errors::GetTargetError;

	#[instrument(skip_all, level = "debug")]
	async fn refresh(&self, options: &RefreshOptions) -> Result<(), Self::RefreshError> {
		GitBasedSource::refresh(self, options).await
	}

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		specifier: &Self::Specifier,
		options: &ResolveOptions,
	) -> Result<ResolveResult<Self::Ref>, Self::ResolveError> {
		let ResolveOptions {
			project,
			target: project_target,
			..
		} = options;

		let Some(IndexFile { meta, entries, .. }) =
			self.read_index_file(&specifier.name, project)?
		else {
			return Err(errors::ResolveError::NotFound(specifier.name.to_string()));
		};

		if !meta.deprecated.is_empty() {
			tracing::warn!("{} is deprecated: {}", specifier.name, meta.deprecated);
		}

		tracing::debug!("{} has {} possible entries", specifier.name, entries.len());

		Ok((
			PackageNames::Pesde(specifier.name.clone()),
			entries
				.into_iter()
				.filter(|(_, entry)| !entry.yanked)
				.filter(|(VersionId(version, target), _)| {
					specifier.version.matches(version)
						&& specifier.target.unwrap_or(*project_target) == *target
				})
				.map(|(id, entry)| {
					(
						id,
						PesdePackageRef {
							index_url: self.repo_url.clone(),
							dependencies: entry.dependencies,
						},
					)
				})
				.collect(),
		))
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter>(
		&self,
		_pkg_ref: &Self::Ref,
		options: &DownloadOptions<R>,
	) -> Result<PackageFs, Self::DownloadError> {
		let DownloadOptions {
			project,
			reporter,
			reqwest,
			id,
			..
		} = options;

		let config = self.config(project).await.map_err(Box::new)?;
		let index_file = project
			.cas_dir()
			.join("index")
			.join(id.name().escaped())
			.join(id.version_id().version().to_string())
			.join(id.version_id().target().to_string());

		match fs::read_to_string(&index_file).await {
			Ok(s) => {
				tracing::debug!("using cached index file for package {id}");

				reporter.report_done();

				return toml::from_str::<PackageFs>(&s).map_err(Into::into);
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(errors::DownloadError::ReadIndex(e)),
		}

		let url = config
			.download()
			.replace("{PACKAGE}", &urlencoding::encode(&id.name().to_string()))
			.replace("{PACKAGE_VERSION}", &urlencoding::encode(&id.version_id().version().to_string()))
			.replace("{PACKAGE_TARGET}", &urlencoding::encode(&id.version_id().target().to_string()));

		let mut request = reqwest.get(&url).header(ACCEPT, "application/octet-stream");

		if let Some(token) = project.auth_config().tokens().get(&self.repo_url) {
			tracing::debug!("using token for {}", self.repo_url);
			request = request.header(AUTHORIZATION, token);
		}

		let response = request.send().await?.error_for_status()?;

		let total_len = response.content_length().unwrap_or(0);
		reporter.report_progress(total_len, 0);

		let mut bytes_downloaded = 0;
		let bytes = response
			.bytes_stream()
			.inspect(|chunk| {
				chunk.as_ref().ok().inspect(|chunk| {
					bytes_downloaded += chunk.len() as u64;
					reporter.report_progress(total_len, bytes_downloaded);
				});
			})
			.map(|result| {
				result.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
			});

		let bytes = StreamReader::new(bytes);

		let mut decoder = async_compression::tokio::bufread::GzipDecoder::new(bytes);
		let mut archive = tokio_tar::Archive::new(&mut decoder);

		let mut entries = BTreeMap::new();

		let mut archive_entries = archive.entries().map_err(errors::DownloadError::Unpack)?;

		while let Some(entry) = archive_entries
			.next()
			.await
			.transpose()
			.map_err(errors::DownloadError::Unpack)?
		{
			let path =
				RelativePathBuf::from_path(entry.path().map_err(errors::DownloadError::Unpack)?)
					.unwrap();
			let name = path.file_name().unwrap_or("");

			if entry.header().entry_type().is_dir() {
				if IGNORED_DIRS.contains(&name) {
					continue;
				}

				entries.insert(path, FsEntry::Directory);

				continue;
			}

			if IGNORED_FILES.contains(&name) {
				continue;
			}

			let hash = store_in_cas(project.cas_dir(), entry)
				.await
				.map_err(errors::DownloadError::Store)?;
			entries.insert(path, FsEntry::File(hash));
		}

		let fs = PackageFs::CAS(entries);

		if let Some(parent) = index_file.parent() {
			fs::create_dir_all(parent)
				.await
				.map_err(errors::DownloadError::WriteIndex)?;
		}

		fs::write(&index_file, toml::to_string(&fs)?)
			.await
			.map_err(errors::DownloadError::WriteIndex)?;

		reporter.report_done();

		Ok(fs)
	}

	#[instrument(skip_all, level = "debug")]
	async fn get_target(
		&self,
		_pkg_ref: &Self::Ref,
		options: &GetTargetOptions,
	) -> Result<Target, Self::GetTargetError> {
		let GetTargetOptions { id, .. } = options;
		#[allow(irrefutable_let_patterns)]
		let PackageNames::Pesde(name) = id.name() else {
			panic!("unexpected package name");
		};

		let Some(IndexFile { mut entries, .. }) = self.read_index_file(name, &options.project)?
		else {
			return Err(errors::GetTargetError::NotFound(name.to_string()));
		};

		let entry = entries
			.remove(id.version_id())
			.ok_or_else(|| errors::GetTargetError::NotFound(name.to_string()))?;

		Ok(entry.target)
	}
}

fn default_archive_size() -> usize {
	4 * 1024 * 1024
}

/// The allowed registries for a package
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum AllowedRegistries {
	/// All registries are allowed
	All(bool),
	/// Only specific registries are allowed
	#[serde(deserialize_with = "crate::util::deserialize_gix_url_hashset")]
	Specific(HashSet<Url>),
}

impl Default for AllowedRegistries {
	fn default() -> Self {
		Self::All(false)
	}
}

// strips .git suffix to allow for more flexible matching
fn simplify_url(mut url: Url) -> Url {
	url.path = url.path.strip_suffix(b".git").unwrap_or(&url.path).into();
	url
}

impl AllowedRegistries {
	fn _is_allowed(&self, url: &Url) -> bool {
		match self {
			Self::All(all) => *all,
			Self::Specific(urls) => urls.contains(url),
		}
	}

	/// Whether the given URL is allowed
	pub fn is_allowed(&self, url: Url) -> bool {
		self._is_allowed(&simplify_url(url))
	}

	/// Whether the given URL is allowed, or is the same as the given URL
	pub fn is_allowed_or_same(&self, this: Url, external: Url) -> bool {
		let this = simplify_url(this);
		let external = simplify_url(external);
		(this == external) || self._is_allowed(&external) || self._is_allowed(&this)
	}
}

/// The configuration for the pesde index
#[derive(Deserialize, Debug, Clone)]
pub struct IndexConfig {
	/// The URL of the API
	pub api: url::Url,
	/// The URL to download packages from
	pub download: Option<String>,
	/// Whether Git is allowed as a source for publishing packages
	#[serde(default)]
	pub git_allowed: AllowedRegistries,
	/// Whether other registries are allowed as a source for publishing packages
	#[serde(default)]
	pub other_registries_allowed: AllowedRegistries,
	/// Whether Wally is allowed as a source for publishing packages
	#[serde(default)]
	pub wally_allowed: AllowedRegistries,
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
	pub fn api(&self) -> &str {
		self.api.as_str().trim_end_matches('/')
	}

	/// The URL to download packages from
	pub fn download(&self) -> String {
		self.download
			.as_deref()
			.unwrap_or("{API_URL}/v0/packages/{PACKAGE}/{PACKAGE_VERSION}/{PACKAGE_TARGET}")
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct IndexFileEntry {
	/// The target for this package
	pub target: Target,
	/// When this package was published
	#[serde(default = "chrono::Utc::now")]
	pub published_at: chrono::DateTime<chrono::Utc>,

	/// The description of this package
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,
	/// The license of this package
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub license: Option<String>,
	/// The authors of this package
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub authors: Vec<String>,
	/// The repository of this package
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub repository: Option<url::Url>,

	/// The documentation for this package
	#[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
	pub docs: BTreeSet<DocEntry>,

	/// Whether this version is yanked
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub yanked: bool,

	/// The dependencies of this package
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub dependencies: BTreeMap<String, (DependencySpecifiers, DependencyType)>,
}

/// The package metadata in the index file
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexMetadata {
	/// Whether this package is deprecated
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub deprecated: String,
}

/// The index file for a package
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct IndexFile {
	/// Any package-wide metadata
	#[serde(default, skip_serializing_if = "crate::util::is_default")]
	pub meta: IndexMetadata,
	/// The entries in the index file
	#[serde(flatten)]
	pub entries: BTreeMap<VersionId, IndexFileEntry>,
}

/// Errors that can occur when interacting with the pesde package source
pub mod errors {
	use thiserror::Error;

	use crate::source::git_index::errors::{ReadFile, TreeError};

	/// Errors that can occur when reading an index file of a pesde package source
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum ReadIndexFileError {
		/// Error reading file
		#[error("error reading file")]
		ReadFile(#[from] ReadFile),

		/// Error opening repository
		#[error("error opening repository")]
		Open(#[from] Box<gix::open::Error>),

		/// Error getting tree
		#[error("error getting tree")]
		Tree(#[from] Box<TreeError>),

		/// Error parsing file
		#[error("error parsing file")]
		Parse(#[from] toml::de::Error),
	}

	/// Errors that can occur when resolving a package from a pesde package source
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum ResolveError {
		/// Package not found in index
		#[error("package {0} not found")]
		NotFound(String),

		/// Error reading index file
		#[error("error reading index file")]
		ReadIndex(#[from] ReadIndexFileError),
	}

	/// Errors that can occur when reading the config file for a pesde package source
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum ConfigError {
		/// Error opening repository
		#[error("error opening repository")]
		Open(#[from] Box<gix::open::Error>),

		/// Error getting tree
		#[error("error getting tree")]
		Tree(#[from] Box<TreeError>),

		/// Error reading file
		#[error("error reading config file")]
		ReadFile(#[from] Box<ReadFile>),

		/// Error parsing config file
		#[error("error parsing config file")]
		Parse(#[from] toml::de::Error),

		/// The config file is missing
		#[error("missing config file for index at {0}")]
		Missing(Box<gix::Url>),
	}

	/// Errors that can occur when downloading a package from a pesde package source
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum DownloadError {
		/// Error reading index file
		#[error("error reading config file")]
		ReadFile(#[from] Box<ConfigError>),

		/// Error downloading package
		#[error("error downloading package")]
		Download(#[from] reqwest::Error),

		/// Error unpacking package
		#[error("error unpacking package")]
		Unpack(#[source] std::io::Error),

		/// Error storing file in CAS
		#[error("error storing file in CAS")]
		Store(#[source] std::io::Error),

		/// Error writing index file
		#[error("error writing index file")]
		WriteIndex(#[source] std::io::Error),

		/// Error serializing index file
		#[error("error serializing index file")]
		SerializeIndex(#[from] toml::ser::Error),

		/// Error deserializing index file
		#[error("error deserializing index file")]
		DeserializeIndex(#[from] toml::de::Error),

		/// Error writing index file
		#[error("error reading index file")]
		ReadIndex(#[source] std::io::Error),
	}

	/// Errors that can occur when getting the target for a package from a pesde package source
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum GetTargetError {
		/// Error reading index file
		#[error("error reading index file")]
		ReadIndex(#[from] ReadIndexFileError),

		/// Package not found in index
		#[error("package `{0}` not found in index")]
		NotFound(String),
	}
}
