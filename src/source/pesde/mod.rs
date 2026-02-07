#![deprecated = "pesde has dropped registries. See https://github.com/pesde-pkg/pesde/issues/59"]
#![expect(deprecated)]
use relative_path::RelativePathBuf;
use reqwest::header::ACCEPT;
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fmt::Debug;
use std::fmt::Display;
use std::hash::Hash;
use std::path::PathBuf;
use std::str::FromStr;

use pkg_ref::PesdePackageRef;
use specifier::PesdeDependencySpecifier;

use crate::GixUrl;
use crate::Project;
use crate::engine::EngineKind;
use crate::manifest::Alias;
use crate::manifest::DependencyType;
use crate::manifest::Manifest;
use crate::names::PackageName;
use crate::reporters::DownloadProgressReporter;
use crate::reporters::response_to_async_read;
use crate::ser_display_deser_fromstr;
use crate::source::DependencySpecifiers;
use crate::source::IGNORED_DIRS;
use crate::source::IGNORED_FILES;
use crate::source::PackageRefs;
use crate::source::PackageSource;
use crate::source::PackageSources;
use crate::source::ResolveResult;
use crate::source::fs::FsEntry;
use crate::source::fs::PackageFs;
use crate::source::fs::store_in_cas;
use crate::source::git_index::GitBasedSource;
use crate::source::git_index::read_file;
use crate::source::git_index::root_tree;
use crate::source::pesde::target::Target;
use crate::source::pesde::target::TargetKind;
use crate::source::traits::DownloadOptions;
use crate::source::traits::GetExportsOptions;
use crate::source::traits::PackageExports;
use crate::source::traits::RefreshOptions;
use crate::source::traits::ResolveOptions;
use crate::util::hash;
use crate::version_matches;
use fs_err::tokio as fs;
use futures::StreamExt as _;
use semver::Version;
use semver::VersionReq;
use tokio::pin;
use tokio::task::spawn_blocking;
use tracing::instrument;

/// The pesde package reference
pub mod pkg_ref;
/// The pesde dependency specifier
pub mod specifier;
/// Targets
pub mod target;

/// The pesde package source
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct PesdePackageSource {
	repo_url: GixUrl,
}
ser_display_deser_fromstr!(PesdePackageSource);

impl Display for PesdePackageSource {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.repo_url)
	}
}

impl FromStr for PesdePackageSource {
	type Err = crate::errors::GixUrlError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self::new)
	}
}

impl GitBasedSource for PesdePackageSource {
	fn path(&self, project: &Project) -> PathBuf {
		project
			.data_dir()
			.join("indices")
			.join(hash(self.as_bytes()))
	}

	fn repo_url(&self) -> &GixUrl {
		&self.repo_url
	}
}

impl PesdePackageSource {
	/// Creates a new pesde package source
	#[must_use]
	pub fn new(repo_url: GixUrl) -> Self {
		Self { repo_url }
	}

	fn as_bytes(&self) -> Vec<u8> {
		self.repo_url.to_string().into_bytes()
	}

	/// Reads the config file
	#[instrument(skip_all, ret(level = "trace"), level = "debug")]
	pub async fn config(&self, project: &Project) -> Result<IndexConfig, errors::ConfigError> {
		let repo_url = self.repo_url.clone();
		let path = self.path(project);

		spawn_blocking(move || {
			let repo = gix::open(&path)?;
			let tree = root_tree(&repo)?;
			let file = read_file(&tree, ["config.toml"])?;

			match file {
				Some(s) => toml::from_str(&s).map_err(Into::into),
				None => Err(errors::ConfigErrorKind::Missing(repo_url).into()),
			}
		})
		.await
		.unwrap()
	}

	/// Reads the index file of a package
	pub async fn read_index_file(
		&self,
		name: PackageName,
		project: &Project,
	) -> Result<Option<IndexFile>, errors::ReadIndexFileError> {
		let path = self.path(project);

		spawn_blocking(move || {
			let (scope, name) = name.as_str();
			let repo = gix::open(&path)?;
			let tree = root_tree(&repo)?;
			let string = match read_file(&tree, [scope, name]) {
				Ok(Some(s)) => s,
				Ok(None) => return Ok(None),
				Err(e) => {
					return Err(errors::ReadIndexFileErrorKind::ReadFile(e).into());
				}
			};

			toml::from_str(&string).map_err(Into::into)
		})
		.await
		.unwrap()
	}
}

impl PackageSource for PesdePackageSource {
	type Specifier = PesdeDependencySpecifier;
	type Ref = PesdePackageRef;
	type RefreshError = errors::RefreshError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetExportsError = errors::GetExportsError;

	#[instrument(skip_all, level = "debug")]
	async fn refresh(&self, options: &RefreshOptions) -> Result<(), Self::RefreshError> {
		GitBasedSource::refresh(self, options).await
	}

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		specifier: &Self::Specifier,
		options: &ResolveOptions,
	) -> Result<ResolveResult, Self::ResolveError> {
		let ResolveOptions { subproject, .. } = options;

		let Some(IndexFile { entries, .. }) = self
			.read_index_file(specifier.name.clone(), subproject.project())
			.await?
		else {
			return Err(errors::ResolveErrorKind::NotFound(specifier.name.clone()).into());
		};

		tracing::debug!("{} has {} possible entries", specifier.name, entries.len());

		let mut suggestions = BTreeSet::new();

		let versions = entries
			.into_iter()
			.filter(|(_, entry)| !entry.yanked)
			.filter(|(v_id, _)| version_matches(&specifier.version, v_id.version()))
			.inspect(|(v_id, _)| {
				suggestions.insert(v_id.target());
			})
			.filter(|(v_id, _)| specifier.target == v_id.target())
			.map(|(v_id, entry)| (v_id.0, entry.dependencies))
			.collect::<BTreeMap<_, _>>();

		if versions.is_empty() {
			return Err(errors::ResolveErrorKind::NoMatchingVersion(
				specifier.clone(),
				specifier.target,
				suggestions,
			)
			.into());
		}

		Ok((
			PackageSources::Pesde(self.clone()),
			PackageRefs::Pesde(PesdePackageRef {
				name: specifier.name.clone(),
				target: specifier.target,
			}),
			versions,
		))
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter>(
		&self,
		pkg_ref: &Self::Ref,
		options: &DownloadOptions<'_, R>,
	) -> Result<PackageFs, Self::DownloadError> {
		let DownloadOptions {
			project,
			reporter,
			reqwest,
			version,
			..
		} = options;

		let config = self.config(project).await?;
		let index_file = project
			.cas_dir()
			.join("index")
			.join(hash(self.as_bytes()))
			.join(pkg_ref.name.escaped())
			.join(version.to_string())
			.join(pkg_ref.target.to_string());

		match fs::read_to_string(&index_file).await {
			Ok(s) => {
				tracing::debug!(
					"using cached index file for package {}@{version} {}",
					pkg_ref.name,
					pkg_ref.target
				);

				reporter.report_done();

				return toml::from_str::<PackageFs>(&s).map_err(Into::into);
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(errors::DownloadErrorKind::ReadIndex(e).into()),
		}

		let url = config
			.download()
			.replace("{PACKAGE}", &urlencoding::encode(&pkg_ref.name.to_string()))
			.replace(
				"{PACKAGE_VERSION}",
				&urlencoding::encode(&version.to_string()),
			)
			.replace(
				"{PACKAGE_TARGET}",
				&urlencoding::encode(&pkg_ref.target.to_string()),
			);

		let mut request = reqwest.get(&url).header(ACCEPT, "application/octet-stream");

		if let Some(token) = project.auth_config().tokens().get(&self.repo_url) {
			tracing::debug!("using token for {}", self.repo_url);
			request = request.header(AUTHORIZATION, token);
		}

		let response = request.send().await?.error_for_status()?;

		let bytes = response_to_async_read(response, reporter.clone());
		pin!(bytes);

		let mut decoder = async_compression::tokio::bufread::GzipDecoder::new(bytes);
		let mut archive = tokio_tar::Archive::new(&mut decoder);

		let mut entries = BTreeMap::new();

		let mut archive_entries = archive
			.entries()
			.map_err(errors::DownloadErrorKind::Unpack)?;

		while let Some(entry) = archive_entries
			.next()
			.await
			.transpose()
			.map_err(errors::DownloadErrorKind::Unpack)?
		{
			let path = RelativePathBuf::from_path(
				entry.path().map_err(errors::DownloadErrorKind::Unpack)?,
			)
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
				.map_err(errors::DownloadErrorKind::Store)?;
			entries.insert(path, FsEntry::File(hash));
		}

		let fs = PackageFs::Cached(entries);

		if let Some(parent) = index_file.parent() {
			fs::create_dir_all(parent)
				.await
				.map_err(errors::DownloadErrorKind::WriteIndex)?;
		}

		fs::write(&index_file, toml::to_string(&fs)?)
			.await
			.map_err(errors::DownloadErrorKind::WriteIndex)?;

		Ok(fs)
	}

	#[instrument(skip_all, level = "debug")]
	async fn get_exports(
		&self,
		pkg_ref: &Self::Ref,
		options: &GetExportsOptions<'_>,
	) -> Result<PackageExports, Self::GetExportsError> {
		let Some(IndexFile { mut entries, .. }) = self
			.read_index_file(pkg_ref.name.clone(), &options.project)
			.await?
		else {
			return Err(errors::GetExportsErrorKind::NotFound(pkg_ref.name.clone()).into());
		};

		let entry = entries
			.remove(&VersionId::new(options.version.clone(), pkg_ref.target))
			.ok_or_else(|| errors::GetExportsErrorKind::NotFound(pkg_ref.name.clone()))?;

		Ok(entry.target.into_exports())
	}
}

fn default_archive_size() -> usize {
	4 * 1024 * 1024
}

/// The configuration for the pesde index
#[derive(Deserialize, Debug, Clone)]
pub struct IndexConfig {
	/// The URL of the API
	pub api: url::Url,
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
		self.api.as_str().trim_end_matches('/')
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
	/// The engines this package supports
	#[serde(default)]
	pub engines: BTreeMap<EngineKind, VersionReq>,

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
	pub repository: Option<url::Url>,

	/// The documentation for this package
	#[serde(default)]
	pub docs: BTreeSet<DocEntry>,

	/// Whether this version is yanked
	#[serde(default)]
	pub yanked: bool,

	/// The dependencies of this package
	#[serde(default)]
	pub dependencies: BTreeMap<Alias, (DependencySpecifiers, DependencyType)>,
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
}

impl Display for VersionId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

/// A pesde v1 (<0.8) manifest
#[derive(Debug, Deserialize)]
pub struct PesdeV1Manifest {
	/// The version
	pub version: Version,
	/// The target
	pub target: Target,
	/// The pesde v2-compatible fields
	#[serde(flatten)]
	pub manifest: Manifest,
	/// Any extra fields
	#[serde(flatten)]
	pub user_defined_fields: HashMap<String, toml::Value>,
}

/// A manifest for either version of pesde
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum PesdeVersionedManifest {
	/// [Manifest]
	V2(Manifest),
	/// [PesdeV1Manifest]
	V1(PesdeV1Manifest),
}

impl PesdeVersionedManifest {
	/// Returns the manifest
	#[must_use]
	pub fn as_manifest(&self) -> &Manifest {
		match self {
			Self::V1(m) => &m.manifest,
			Self::V2(m) => m,
		}
	}

	/// Returns the manifest
	#[must_use]
	pub fn into_manifest(self) -> Manifest {
		match self {
			Self::V1(m) => m.manifest,
			Self::V2(m) => m,
		}
	}

	/// Returns the exports for this manifest
	#[must_use]
	pub fn as_exports(&self) -> PackageExports {
		match self {
			Self::V1(m) => m.target.clone().into_exports(),
			Self::V2(m) => m.as_exports(),
		}
	}
}

/// Errors that can occur when interacting with the pesde package source
pub mod errors {
	use std::collections::BTreeSet;

	use itertools::Itertools as _;
	use thiserror::Error;

	use super::target::TargetKind;
	use crate::GixUrl;
	use crate::names::PackageName;
	use crate::source::git_index::errors::ReadFile;
	use crate::source::git_index::errors::TreeError;
	use crate::source::pesde::specifier::PesdeDependencySpecifier;

	/// Errors that can occur when reading an index file of a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ReadIndexFileError))]
	#[non_exhaustive]
	pub enum ReadIndexFileErrorKind {
		/// Error reading file
		#[error("error reading file")]
		ReadFile(#[from] ReadFile),

		/// Error opening repository
		#[error("error opening repository")]
		Open(#[from] gix::open::Error),

		/// Error getting tree
		#[error("error getting tree")]
		Tree(#[from] TreeError),

		/// Error parsing file
		#[error("error parsing file")]
		Parse(#[from] toml::de::Error),
	}

	/// Errors that can occur when refreshing the pesde package source
	pub type RefreshError = crate::source::git_index::errors::RefreshError;

	/// Errors that can occur when resolving a package from a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ResolveError))]
	#[non_exhaustive]
	pub enum ResolveErrorKind {
		/// Package not found in index
		#[error("package `{0}` not found")]
		NotFound(PackageName),

		// custom error to provide the user with target suggestions
		/// No matching version was found for a specifier
		#[error("no matching version found for {0} {1}. available targets: {suggestions}", suggestions = .2.iter().format(", "))]
		NoMatchingVersion(PesdeDependencySpecifier, TargetKind, BTreeSet<TargetKind>),

		/// Error reading index file
		#[error("error reading index file")]
		ReadIndex(#[from] ReadIndexFileError),
	}

	/// Errors that can occur when reading the config file for a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ConfigError))]
	#[non_exhaustive]
	pub enum ConfigErrorKind {
		/// Error opening repository
		#[error("error opening repository")]
		Open(#[from] gix::open::Error),

		/// Error getting tree
		#[error("error getting tree")]
		Tree(#[from] TreeError),

		/// Error reading file
		#[error("error reading config file")]
		ReadFile(#[from] ReadFile),

		/// Error parsing config file
		#[error("error parsing config file")]
		Parse(#[from] toml::de::Error),

		/// The config file is missing
		#[error("missing config file for index at {0}")]
		Missing(GixUrl),
	}

	/// Errors that can occur when downloading a package from a pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		/// Error reading index file
		#[error("error reading config file")]
		ReadFile(#[from] ConfigError),

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
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GetExportsError))]
	#[non_exhaustive]
	pub enum GetExportsErrorKind {
		/// Error reading index file
		#[error("error reading index file")]
		ReadIndex(#[from] ReadIndexFileError),

		/// Package not found in index
		#[error("package `{0}` not found in index")]
		NotFound(PackageName),
	}

	/// Errors that can occur when parsing a version ID
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = VersionIdParseError))]
	pub enum VersionIdParseErrorKind {
		/// The version ID is malformed
		#[error("malformed version ID `{0}`")]
		Malformed(String),

		/// Error parsing version
		#[error("error parsing version")]
		Version(#[from] semver::Error),

		/// Error parsing target
		#[error("error parsing target")]
		Target(#[from] super::target::errors::TargetKindFromStr),
	}
}
