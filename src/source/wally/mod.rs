use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};

use gix::Url;
use relative_path::RelativePathBuf;
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use tempfile::tempdir;

use crate::{
    manifest::target::{Target, TargetKind},
    names::PackageNames,
    source::{
        fs::{store_reader_in_cas, FSEntry, PackageFS},
        git_index::GitBasedSource,
        traits::PackageSource,
        version_id::VersionId,
        wally::{compat_util::get_target, manifest::WallyManifest, pkg_ref::WallyPackageRef},
        IGNORED_DIRS, IGNORED_FILES,
    },
    util::hash,
    Project,
};

pub(crate) mod compat_util;
pub(crate) mod manifest;
/// The Wally package reference
pub mod pkg_ref;
/// The Wally dependency specifier
pub mod specifier;

/// The Wally package source
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct WallyPackageSource {
    repo_url: Url,
}

impl GitBasedSource for WallyPackageSource {
    fn path(&self, project: &Project) -> PathBuf {
        project
            .data_dir
            .join("wally_indices")
            .join(hash(self.as_bytes()))
    }

    fn repo_url(&self) -> &Url {
        &self.repo_url
    }
}

impl WallyPackageSource {
    /// Creates a new Wally package source
    pub fn new(repo_url: Url) -> Self {
        Self { repo_url }
    }

    fn as_bytes(&self) -> Vec<u8> {
        self.repo_url.to_bstring().to_vec()
    }

    /// Reads the config file
    pub fn config(&self, project: &Project) -> Result<WallyIndexConfig, errors::ConfigError> {
        let file = self
            .read_file(["config.json"], project, None)
            .map_err(Box::new)?;

        let string = match file {
            Some(s) => s,
            None => {
                return Err(errors::ConfigError::Missing(Box::new(
                    self.repo_url.clone(),
                )))
            }
        };

        serde_json::from_str(&string).map_err(Into::into)
    }
}

impl PackageSource for WallyPackageSource {
    type Specifier = specifier::WallyDependencySpecifier;
    type Ref = WallyPackageRef;
    type RefreshError = crate::source::git_index::errors::RefreshError;
    type ResolveError = errors::ResolveError;
    type DownloadError = errors::DownloadError;

    fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
        GitBasedSource::refresh(self, project)
    }

    fn resolve(
        &self,
        specifier: &Self::Specifier,
        project: &Project,
        _package_target: TargetKind,
    ) -> Result<crate::source::ResolveResult<Self::Ref>, Self::ResolveError> {
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

        let entries: Vec<WallyManifest> = string
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()
            .map_err(|e| Self::ResolveError::Parse(specifier.name.to_string(), e))?;

        log::debug!("{} has {} possible entries", specifier.name, entries.len());

        Ok((
            PackageNames::Wally(specifier.name.clone()),
            entries
                .into_iter()
                .filter(|manifest| specifier.version.matches(&manifest.package.version))
                .map(|manifest| {
                    Ok((
                        VersionId(manifest.package.version.clone(), TargetKind::Roblox),
                        WallyPackageRef {
                            name: specifier.name.clone(),
                            index_url: self.repo_url.clone(),
                            dependencies: manifest.all_dependencies().map_err(|e| {
                                Self::ResolveError::AllDependencies(specifier.to_string(), e)
                            })?,
                            version: manifest.package.version,
                        },
                    ))
                })
                .collect::<Result<_, Self::ResolveError>>()?,
        ))
    }

    fn download(
        &self,
        pkg_ref: &Self::Ref,
        project: &Project,
        reqwest: &reqwest::blocking::Client,
    ) -> Result<(PackageFS, Target), Self::DownloadError> {
        let config = self.config(project).map_err(Box::new)?;
        let index_file = project
            .cas_dir
            .join("wally_index")
            .join(pkg_ref.name.escaped())
            .join(pkg_ref.version.to_string());

        let tempdir = match fs_err::read_to_string(&index_file) {
            Ok(s) => {
                log::debug!(
                    "using cached index file for package {}@{}",
                    pkg_ref.name,
                    pkg_ref.version
                );

                let tempdir = tempdir()?;
                let fs = toml::from_str::<PackageFS>(&s)?;

                fs.write_to(&tempdir, project.cas_dir(), false)?;

                return Ok((fs, get_target(project, &tempdir)?));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => tempdir()?,
            Err(e) => return Err(errors::DownloadError::ReadIndex(e)),
        };

        let (scope, name) = pkg_ref.name.as_str();

        let url = format!(
            "{}/v1/package-contents/{scope}/{name}/{}",
            config.api.as_str().trim_end_matches('/'),
            pkg_ref.version
        );

        let mut request = reqwest.get(&url).header(
            "Wally-Version",
            std::env::var("PESDE_WALLY_VERSION")
                .as_deref()
                .unwrap_or("0.3.2"),
        );

        if let Some(token) = project.auth_config.tokens().get(&self.repo_url) {
            log::debug!("using token for {}", self.repo_url);
            request = request.header(AUTHORIZATION, token);
        }

        let response = request.send()?.error_for_status()?;
        let bytes = response.bytes()?;

        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
        archive.extract(tempdir.path())?;

        let mut entries = BTreeMap::new();

        let mut dir_entries = fs_err::read_dir(tempdir.path())?.collect::<VecDeque<_>>();
        while let Some(entry) = dir_entries.pop_front() {
            let entry = entry?;
            let path =
                RelativePathBuf::from_path(entry.path().strip_prefix(tempdir.path())?).unwrap();

            if entry.file_type()?.is_dir() {
                if IGNORED_DIRS.contains(&path.as_str()) {
                    continue;
                }

                entries.insert(path, FSEntry::Directory);
                dir_entries.extend(fs_err::read_dir(entry.path())?);

                continue;
            }

            if IGNORED_FILES.contains(&path.as_str()) {
                continue;
            }

            let mut file = fs_err::File::open(entry.path())?;
            let hash = store_reader_in_cas(project.cas_dir(), &mut file)?;
            entries.insert(path, FSEntry::File(hash));
        }

        let fs = PackageFS::CAS(entries);

        if let Some(parent) = index_file.parent() {
            fs_err::create_dir_all(parent).map_err(errors::DownloadError::WriteIndex)?;
        }

        fs_err::write(&index_file, toml::to_string(&fs)?)
            .map_err(errors::DownloadError::WriteIndex)?;

        Ok((fs, get_target(project, &tempdir)?))
    }
}

/// A Wally index config
#[derive(Debug, Clone, Deserialize)]
pub struct WallyIndexConfig {
    api: url::Url,
}

/// Errors that can occur when interacting with a Wally package source
pub mod errors {
    use thiserror::Error;

    use crate::source::git_index::errors::ReadFile;

    /// Errors that can occur when resolving a package from a Wally package source
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
        Parse(String, #[source] serde_json::Error),

        /// Error parsing file for package as utf8
        #[error("error parsing file for {0} to utf8")]
        Utf8(String, #[source] std::string::FromUtf8Error),

        /// Error parsing all dependencies
        #[error("error parsing all dependencies for {0}")]
        AllDependencies(
            String,
            #[source] crate::manifest::errors::AllDependenciesError,
        ),
    }

    /// Errors that can occur when reading the config file for a Wally package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum ConfigError {
        /// Error reading file
        #[error("error reading config file")]
        ReadFile(#[from] Box<ReadFile>),

        /// Error parsing config file
        #[error("error parsing config file")]
        Parse(#[from] serde_json::Error),

        /// The config file is missing
        #[error("missing config file for index at {0}")]
        Missing(Box<gix::Url>),
    }

    /// Errors that can occur when downloading a package from a Wally package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum DownloadError {
        /// Error reading index file
        #[error("error reading config file")]
        ReadFile(#[from] Box<ConfigError>),

        /// Error downloading package
        #[error("error downloading package")]
        Download(#[from] reqwest::Error),

        /// Error deserializing index file
        #[error("error deserializing index file")]
        Deserialize(#[from] toml::de::Error),

        /// Error reading index file
        #[error("error reading index file")]
        ReadIndex(#[source] std::io::Error),

        /// Error decompressing archive
        #[error("error decompressing archive")]
        Decompress(#[from] zip::result::ZipError),

        /// Error interacting with the filesystem
        #[error("error interacting with the filesystem")]
        Io(#[from] std::io::Error),

        /// Error stripping prefix from path
        #[error("error stripping prefix from path")]
        StripPrefix(#[from] std::path::StripPrefixError),

        /// Error serializing index file
        #[error("error serializing index file")]
        SerializeIndex(#[from] toml::ser::Error),

        /// Error getting lib path
        #[error("error getting lib path")]
        LibPath(#[from] crate::source::wally::compat_util::errors::FindLibPathError),

        /// Error writing index file
        #[error("error writing index file")]
        WriteIndex(#[source] std::io::Error),
    }
}
