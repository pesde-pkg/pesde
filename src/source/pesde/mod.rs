use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    hash::Hash,
    path::PathBuf,
};

use gix::Url;
use relative_path::RelativePathBuf;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::{Deserialize, Serialize};

use pkg_ref::PesdePackageRef;
use specifier::PesdeDependencySpecifier;

use crate::{
    manifest::{
        target::{Target, TargetKind},
        DependencyType,
    },
    names::{PackageName, PackageNames},
    source::{
        fs::{store_in_cas, FSEntry, PackageFS},
        git_index::GitBasedSource,
        DependencySpecifiers, PackageSource, ResolveResult, VersionId, IGNORED_DIRS, IGNORED_FILES,
    },
    util::hash,
    Project,
};
use fs_err::tokio as fs;
use futures::StreamExt;

// TODO: make more of these functions async

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
        project.data_dir.join("indices").join(hash(self.as_bytes()))
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
    pub fn config(&self, project: &Project) -> Result<IndexConfig, errors::ConfigError> {
        let file = self
            .read_file(["config.toml"], project, None)
            .map_err(Box::new)?;

        let string = match file {
            Some(s) => s,
            None => {
                return Err(errors::ConfigError::Missing(Box::new(
                    self.repo_url.clone(),
                )))
            }
        };

        toml::from_str(&string).map_err(Into::into)
    }

    /// Reads all packages from the index
    pub fn all_packages(
        &self,
        project: &Project,
    ) -> Result<BTreeMap<PackageName, IndexFile>, errors::AllPackagesError> {
        let path = self.path(project);

        let repo = match gix::open(&path) {
            Ok(repo) => repo,
            Err(e) => return Err(errors::AllPackagesError::Open(path, Box::new(e))),
        };

        let tree = match self.tree(&repo) {
            Ok(tree) => tree,
            Err(e) => return Err(errors::AllPackagesError::Tree(path, Box::new(e))),
        };

        let mut packages = BTreeMap::<PackageName, IndexFile>::new();

        for entry in tree.iter() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => return Err(errors::AllPackagesError::Decode(path, e)),
            };

            let object = match entry.object() {
                Ok(object) => object,
                Err(e) => return Err(errors::AllPackagesError::Convert(path, e)),
            };

            // directories will be trees, and files will be blobs
            if !matches!(object.kind, gix::object::Kind::Tree) {
                continue;
            }

            let package_scope = entry.filename().to_string();

            for inner_entry in object.into_tree().iter() {
                let inner_entry = match inner_entry {
                    Ok(entry) => entry,
                    Err(e) => return Err(errors::AllPackagesError::Decode(path, e)),
                };

                let object = match inner_entry.object() {
                    Ok(object) => object,
                    Err(e) => return Err(errors::AllPackagesError::Convert(path, e)),
                };

                if !matches!(object.kind, gix::object::Kind::Blob) {
                    continue;
                }

                let package_name = inner_entry.filename().to_string();

                if package_name == SCOPE_INFO_FILE {
                    continue;
                }

                let blob = object.into_blob();
                let string = String::from_utf8(blob.data.clone())
                    .map_err(|e| errors::AllPackagesError::Utf8(package_name.to_string(), e))?;

                let file: IndexFile = match toml::from_str(&string) {
                    Ok(file) => file,
                    Err(e) => {
                        return Err(errors::AllPackagesError::Deserialize(
                            package_name,
                            path,
                            Box::new(e),
                        ))
                    }
                };

                // if this panics, it's an issue with the index.
                let name = format!("{package_scope}/{package_name}").parse().unwrap();

                packages.insert(name, file);
            }
        }

        Ok(packages)
    }

    /// The git2 repository for the index
    #[cfg(feature = "git2")]
    pub fn repo_git2(&self, project: &Project) -> Result<git2::Repository, git2::Error> {
        let path = self.path(project);

        git2::Repository::open_bare(&path)
    }
}

impl PackageSource for PesdePackageSource {
    type Specifier = PesdeDependencySpecifier;
    type Ref = PesdePackageRef;
    type RefreshError = crate::source::git_index::errors::RefreshError;
    type ResolveError = errors::ResolveError;
    type DownloadError = errors::DownloadError;

    async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
        GitBasedSource::refresh(self, project).await
    }

    async fn resolve(
        &self,
        specifier: &Self::Specifier,
        project: &Project,
        package_target: TargetKind,
    ) -> Result<ResolveResult<Self::Ref>, Self::ResolveError> {
        let (scope, name) = specifier.name.as_str();
        let string = match self.read_file([scope, name], project, None) {
            Ok(Some(s)) => s,
            Ok(None) => return Err(Self::ResolveError::NotFound(specifier.name.to_string())),
            Err(e) => {
                return Err(Self::ResolveError::Read(
                    specifier.name.to_string(),
                    Box::new(e),
                ))
            }
        };

        let entries: IndexFile = toml::from_str(&string)
            .map_err(|e| Self::ResolveError::Parse(specifier.name.to_string(), e))?;

        log::debug!("{} has {} possible entries", specifier.name, entries.len());

        Ok((
            PackageNames::Pesde(specifier.name.clone()),
            entries
                .into_iter()
                .filter(|(VersionId(version, target), _)| {
                    specifier.version.matches(version)
                        && specifier.target.unwrap_or(package_target) == *target
                })
                .map(|(id, entry)| {
                    let version = id.version().clone();

                    (
                        id,
                        PesdePackageRef {
                            name: specifier.name.clone(),
                            version,
                            index_url: self.repo_url.clone(),
                            dependencies: entry.dependencies,
                            target: entry.target,
                        },
                    )
                })
                .collect(),
        ))
    }

    async fn download(
        &self,
        pkg_ref: &Self::Ref,
        project: &Project,
        reqwest: &reqwest::Client,
    ) -> Result<(PackageFS, Target), Self::DownloadError> {
        let config = self.config(project).map_err(Box::new)?;
        let index_file = project
            .cas_dir
            .join("index")
            .join(pkg_ref.name.escaped())
            .join(pkg_ref.version.to_string())
            .join(pkg_ref.target.to_string());

        match fs::read_to_string(&index_file).await {
            Ok(s) => {
                log::debug!(
                    "using cached index file for package {}@{} {}",
                    pkg_ref.name,
                    pkg_ref.version,
                    pkg_ref.target
                );
                return Ok((toml::from_str::<PackageFS>(&s)?, pkg_ref.target.clone()));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(errors::DownloadError::ReadIndex(e)),
        }

        let url = config
            .download()
            .replace("{PACKAGE}", &pkg_ref.name.to_string().replace("/", "%2F"))
            .replace("{PACKAGE_VERSION}", &pkg_ref.version.to_string())
            .replace("{PACKAGE_TARGET}", &pkg_ref.target.to_string());

        let mut request = reqwest.get(&url).header(ACCEPT, "application/octet-stream");

        if let Some(token) = project.auth_config.tokens().get(&self.repo_url) {
            log::debug!("using token for {}", self.repo_url);
            request = request.header(AUTHORIZATION, token);
        }

        let response = request.send().await?.error_for_status()?;
        let bytes = response.bytes().await?;

        let mut decoder = async_compression::tokio::bufread::GzipDecoder::new(bytes.as_ref());
        let mut archive = tokio_tar::Archive::new(&mut decoder);

        let mut entries = BTreeMap::new();

        while let Some(entry) = archive
            .entries()
            .map_err(errors::DownloadError::Unpack)?
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

                entries.insert(path, FSEntry::Directory);

                continue;
            }

            if IGNORED_FILES.contains(&name) {
                continue;
            }

            let hash = store_in_cas(project.cas_dir(), entry, |_| async { Ok(()) })
                .await
                .map_err(errors::DownloadError::Store)?;
            entries.insert(path, FSEntry::File(hash));
        }

        let fs = PackageFS::CAS(entries);

        if let Some(parent) = index_file.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(errors::DownloadError::WriteIndex)?;
        }

        fs::write(&index_file, toml::to_string(&fs)?)
            .await
            .map_err(errors::DownloadError::WriteIndex)?;

        Ok((fs, pkg_ref.target.clone()))
    }
}

fn default_archive_size() -> usize {
    4 * 1024 * 1024
}

/// The configuration for the pesde index
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct IndexConfig {
    /// The URL of the API
    pub api: url::Url,
    /// The URL to download packages from
    pub download: Option<String>,
    /// Whether Git is allowed as a source for publishing packages
    #[serde(default)]
    pub git_allowed: bool,
    /// Whether other registries are allowed as a source for publishing packages
    #[serde(default)]
    pub other_registries_allowed: bool,
    /// Whether Wally is allowed as a source for publishing packages
    #[serde(default)]
    pub wally_allowed: bool,
    /// The OAuth client ID for GitHub
    #[serde(default)]
    pub github_oauth_client_id: Option<String>,
    /// The maximum size of an archive in bytes
    #[serde(default = "default_archive_size")]
    pub max_archive_size: usize,
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

    /// The dependencies of this package
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, (DependencySpecifiers, DependencyType)>,
}

/// The index file for a package
pub type IndexFile = BTreeMap<VersionId, IndexFileEntry>;

/// Errors that can occur when interacting with the pesde package source
pub mod errors {
    use std::path::PathBuf;

    use thiserror::Error;

    use crate::source::git_index::errors::{ReadFile, TreeError};

    /// Errors that can occur when resolving a package from a pesde package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum ResolveError {
        /// Error interacting with the filesystem
        #[error("error interacting with the filesystem")]
        Io(#[from] std::io::Error),

        /// Package not found in index
        #[error("package {0} not found")]
        NotFound(String),

        /// Error reading file for package
        #[error("error reading file for {0}")]
        Read(String, #[source] Box<ReadFile>),

        /// Error parsing file for package
        #[error("error parsing file for {0}")]
        Parse(String, #[source] toml::de::Error),

        /// Error parsing file for package as utf8
        #[error("error parsing file for {0} to utf8")]
        Utf8(String, #[source] std::string::FromUtf8Error),
    }

    /// Errors that can occur when reading the config file for a pesde package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum ConfigError {
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

    /// Errors that can occur when reading all packages from a pesde package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum AllPackagesError {
        /// Error opening the repository
        #[error("error opening repository at {0}")]
        Open(PathBuf, #[source] Box<gix::open::Error>),

        /// Error reading tree from repository
        #[error("error getting tree from repository at {0}")]
        Tree(PathBuf, #[source] Box<TreeError>),

        /// Error decoding entry in repository
        #[error("error decoding entry in repository at {0}")]
        Decode(PathBuf, #[source] gix::objs::decode::Error),

        /// Error converting entry in repository
        #[error("error converting entry in repository at {0}")]
        Convert(PathBuf, #[source] gix::object::find::existing::Error),

        /// Error deserializing file in repository
        #[error("error deserializing file {0} in repository at {1}")]
        Deserialize(String, PathBuf, #[source] Box<toml::de::Error>),

        /// Error parsing file in repository as utf8
        #[error("error parsing file for {0} as utf8")]
        Utf8(String, #[source] std::string::FromUtf8Error),
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
}
