#![deprecated = "pesde has dropped registries. See https://github.com/pesde-pkg/pesde/issues/59"]
#![expect(deprecated)]
use relative_path::RelativePathBuf;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use std::{
	collections::{BTreeMap, BTreeSet, HashMap},
	fmt::{Debug, Display},
	hash::Hash,
	path::PathBuf,
	str::FromStr,
};

use pkg_ref::PesdePackageRef;
use specifier::PesdeDependencySpecifier;

use crate::{
	GixUrl, Project,
	engine::EngineKind,
	manifest::{
		Alias, DependencyType, Manifest,
		target::{Target, TargetKind},
	},
	names::PackageName,
	reporters::{DownloadProgressReporter, response_to_async_read},
	ser_display_deser_fromstr,
	source::{
		DependencySpecifiers, IGNORED_DIRS, IGNORED_FILES, PackageSource, PackageSources,
		ResolveResult, VersionId,
		fs::{FsEntry, PackageFs, store_in_cas},
		git_index::{GitBasedSource, read_file, root_tree},
		refs::PackageRefs,
		traits::{DownloadOptions, GetTargetOptions, RefreshOptions, ResolveOptions},
	},
	util::hash,
	version_matches,
};
use fs_err::tokio as fs;
use futures::StreamExt as _;
use semver::{Version, VersionReq};
use tokio::{pin, task::spawn_blocking};
use tracing::instrument;

/// The pesde package reference
pub mod pkg_ref;
/// The pesde dependency specifier
pub mod specifier;

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
			let repo = gix::open(&path).map_err(Box::new)?;
			let tree = root_tree(&repo).map_err(Box::new)?;
			let file = read_file(&tree, ["config.toml"]).map_err(Box::new)?;

			match file {
				Some(s) => toml::from_str(&s).map_err(Into::into),
				None => Err(errors::ConfigError::Missing(repo_url)),
			}
		})
		.await
		.unwrap()
	}

	/// Reads the index file of a package
	pub async fn read_index_file(
		&self,
		name: &PackageName,
		project: &Project,
	) -> Result<Option<IndexFile>, errors::ReadIndexFileError> {
		let path = self.path(project);
		let name = name.clone();

		spawn_blocking(move || {
			let (scope, name) = name.as_str();
			let repo = gix::open(&path).map_err(Box::new)?;
			let tree = root_tree(&repo).map_err(Box::new)?;
			let string = match read_file(&tree, [scope, name]) {
				Ok(Some(s)) => s,
				Ok(None) => return Ok(None),
				Err(e) => {
					return Err(errors::ReadIndexFileError::ReadFile(e));
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
	) -> Result<ResolveResult, Self::ResolveError> {
		let ResolveOptions {
			project,
			target: project_target,
			loose_target,
			..
		} = options;

		let Some(IndexFile { entries, .. }) =
			self.read_index_file(&specifier.name, project).await?
		else {
			return Err(errors::ResolveError::NotFound(specifier.name.clone()));
		};

		tracing::debug!("{} has {} possible entries", specifier.name, entries.len());

		let suggestions = entries
			.iter()
			.filter(|(_, entry)| !entry.yanked)
			.filter(|(v_id, _)| version_matches(&specifier.version, v_id.version()))
			.map(|(v_id, _)| v_id.target())
			.collect();

		let specifier_target = specifier.target.unwrap_or(*project_target);

		Ok((
			PackageSources::Pesde(self.clone()),
			PackageRefs::Pesde(PesdePackageRef {
				name: specifier.name.clone(),
			}),
			entries
				.into_iter()
				.filter(|(_, entry)| !entry.yanked)
				.filter(|(v_id, _)| version_matches(&specifier.version, v_id.version()))
				.filter(|(v_id, _)| {
					// we want anything which might contain bins, scripts (so not Roblox)
					if *loose_target && specifier_target == TargetKind::Luau {
						!matches!(v_id.target(), TargetKind::Roblox | TargetKind::RobloxServer)
					} else {
						specifier_target == v_id.target()
					}
				})
				.map(|(v_id, entry)| (v_id, entry.dependencies))
				.collect(),
			suggestions,
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
			version_id,
			..
		} = options;

		let config = self.config(project).await.map_err(Box::new)?;
		let index_file = project
			.cas_dir()
			.join("index")
			.join(hash(self.as_bytes()))
			.join(pkg_ref.name.escaped())
			.join(version_id.version().to_string())
			.join(version_id.target().to_string());

		match fs::read_to_string(&index_file).await {
			Ok(s) => {
				tracing::debug!(
					"using cached index file for package {}@{version_id}",
					pkg_ref.name
				);

				reporter.report_done();

				return toml::from_str::<PackageFs>(&s).map_err(Into::into);
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(errors::DownloadError::ReadIndex(e)),
		}

		let url = config
			.download()
			.replace("{PACKAGE}", &urlencoding::encode(&pkg_ref.name.to_string()))
			.replace(
				"{PACKAGE_VERSION}",
				&urlencoding::encode(&version_id.version().to_string()),
			)
			.replace(
				"{PACKAGE_TARGET}",
				&urlencoding::encode(&version_id.target().to_string()),
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

		let fs = PackageFs::Cached(entries);

		if let Some(parent) = index_file.parent() {
			fs::create_dir_all(parent)
				.await
				.map_err(errors::DownloadError::WriteIndex)?;
		}

		fs::write(&index_file, toml::to_string(&fs)?)
			.await
			.map_err(errors::DownloadError::WriteIndex)?;

		Ok(fs)
	}

	#[instrument(skip_all, level = "debug")]
	async fn get_target(
		&self,
		pkg_ref: &Self::Ref,
		options: &GetTargetOptions<'_>,
	) -> Result<Target, Self::GetTargetError> {
		let Some(IndexFile { mut entries, .. }) = self
			.read_index_file(&pkg_ref.name, &options.project)
			.await?
		else {
			return Err(errors::GetTargetError::NotFound(pkg_ref.name.clone()));
		};

		let entry = entries
			.remove(options.version_id)
			.ok_or_else(|| errors::GetTargetError::NotFound(pkg_ref.name.clone()))?;

		Ok(entry.target)
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

/// A pesde v1 (<0.8) manifest
#[derive(Debug, Deserialize)]
pub struct PesdeV1Manifest {
	/// The version
	pub version: Version,
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
}

/// Errors that can occur when interacting with the pesde package source
pub mod errors {
	use thiserror::Error;

	use crate::{
		GixUrl,
		names::PackageName,
		source::git_index::errors::{ReadFile, TreeError},
	};

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
		#[error("package `{0}` not found")]
		NotFound(PackageName),

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
		Missing(GixUrl),
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
		NotFound(PackageName),
	}
}
