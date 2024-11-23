#![deny(missing_docs)]
//! A package manager for the Luau programming language, supporting multiple runtimes including Roblox and Lune.
//! pesde has its own registry, however it can also use Wally, and Git repositories as package sources.
//! It has been designed with multiple targets in mind, namely Roblox, Lune, and Luau.

use crate::{
    lockfile::Lockfile,
    manifest::Manifest,
    source::{traits::PackageSource, PackageSources},
};
use async_stream::stream;
use fs_err::tokio as fs;
use futures::{future::try_join_all, Stream};
use gix::sec::identity::Account;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

/// Downloading packages
pub mod download;
/// Linking packages
pub mod linking;
/// Lockfile
pub mod lockfile;
/// Manifest
pub mod manifest;
/// Package names
pub mod names;
/// Patching packages
#[cfg(feature = "patches")]
pub mod patches;
/// Resolving packages
pub mod resolver;
/// Running scripts
pub mod scripts;
/// Package sources
pub mod source;
pub(crate) mod util;

/// The name of the manifest file
pub const MANIFEST_FILE_NAME: &str = "pesde.toml";
/// The name of the lockfile
pub const LOCKFILE_FILE_NAME: &str = "pesde.lock";
/// The name of the default index
pub const DEFAULT_INDEX_NAME: &str = "default";
/// The name of the packages container
pub const PACKAGES_CONTAINER_NAME: &str = ".pesde";
pub(crate) const LINK_LIB_NO_FILE_FOUND: &str = "____pesde_no_export_file_found";

/// Struct containing the authentication configuration
#[derive(Debug, Default, Clone)]
pub struct AuthConfig {
    tokens: HashMap<gix::Url, String>,
    git_credentials: Option<Account>,
}

impl AuthConfig {
    /// Create a new `AuthConfig`
    pub fn new() -> Self {
        AuthConfig::default()
    }

    /// Set the tokens
    pub fn with_tokens<I: IntoIterator<Item = (gix::Url, S)>, S: AsRef<str>>(
        mut self,
        tokens: I,
    ) -> Self {
        self.tokens = tokens
            .into_iter()
            .map(|(url, s)| (url, s.as_ref().to_string()))
            .collect();
        self
    }

    /// Set the git credentials
    pub fn with_git_credentials(mut self, git_credentials: Option<Account>) -> Self {
        self.git_credentials = git_credentials;
        self
    }

    /// Get the tokens
    pub fn tokens(&self) -> &HashMap<gix::Url, String> {
        &self.tokens
    }

    /// Get the git credentials
    pub fn git_credentials(&self) -> Option<&Account> {
        self.git_credentials.as_ref()
    }
}

/// The main struct of the pesde library, representing a project
#[derive(Debug, Clone)]
pub struct Project {
    package_dir: PathBuf,
    workspace_dir: Option<PathBuf>,
    data_dir: PathBuf,
    auth_config: AuthConfig,
    cas_dir: PathBuf,
}

impl Project {
    /// Create a new `Project`
    pub fn new<P: AsRef<Path>, Q: AsRef<Path>, R: AsRef<Path>, S: AsRef<Path>>(
        package_dir: P,
        workspace_dir: Option<Q>,
        data_dir: R,
        cas_dir: S,
        auth_config: AuthConfig,
    ) -> Self {
        Project {
            package_dir: package_dir.as_ref().to_path_buf(),
            workspace_dir: workspace_dir.map(|d| d.as_ref().to_path_buf()),
            data_dir: data_dir.as_ref().to_path_buf(),
            auth_config,
            cas_dir: cas_dir.as_ref().to_path_buf(),
        }
    }

    /// The directory of the package
    pub fn package_dir(&self) -> &Path {
        &self.package_dir
    }

    /// The directory of the workspace this package belongs to, if any
    pub fn workspace_dir(&self) -> Option<&Path> {
        self.workspace_dir.as_deref()
    }

    /// The directory to store general-purpose data
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The authentication configuration
    pub fn auth_config(&self) -> &AuthConfig {
        &self.auth_config
    }

    /// The CAS (content-addressable storage) directory
    pub fn cas_dir(&self) -> &Path {
        &self.cas_dir
    }

    /// Read the manifest file
    pub async fn read_manifest(&self) -> Result<String, errors::ManifestReadError> {
        let string = fs::read_to_string(self.package_dir.join(MANIFEST_FILE_NAME)).await?;
        Ok(string)
    }

    /// Deserialize the manifest file
    pub async fn deser_manifest(&self) -> Result<Manifest, errors::ManifestReadError> {
        let string = fs::read_to_string(self.package_dir.join(MANIFEST_FILE_NAME)).await?;
        Ok(toml::from_str(&string)?)
    }

    /// Write the manifest file
    pub async fn write_manifest<S: AsRef<[u8]>>(&self, manifest: S) -> Result<(), std::io::Error> {
        fs::write(self.package_dir.join(MANIFEST_FILE_NAME), manifest.as_ref()).await
    }

    /// Deserialize the lockfile
    pub async fn deser_lockfile(&self) -> Result<Lockfile, errors::LockfileReadError> {
        let string = fs::read_to_string(self.package_dir.join(LOCKFILE_FILE_NAME)).await?;
        Ok(toml::from_str(&string)?)
    }

    /// Write the lockfile
    pub async fn write_lockfile(
        &self,
        lockfile: Lockfile,
    ) -> Result<(), errors::LockfileWriteError> {
        let string = toml::to_string(&lockfile)?;
        fs::write(self.package_dir.join(LOCKFILE_FILE_NAME), string).await?;
        Ok(())
    }

    /// Get the workspace members
    pub async fn workspace_members<P: AsRef<Path>>(
        &self,
        dir: P,
    ) -> Result<
        impl Stream<Item = Result<(PathBuf, Manifest), errors::WorkspaceMembersError>>,
        errors::WorkspaceMembersError,
    > {
        let dir = dir.as_ref().to_path_buf();
        let manifest = fs::read_to_string(dir.join(MANIFEST_FILE_NAME))
            .await
            .map_err(errors::WorkspaceMembersError::ManifestMissing)?;
        let manifest = toml::from_str::<Manifest>(&manifest).map_err(|e| {
            errors::WorkspaceMembersError::ManifestDeser(dir.to_path_buf(), Box::new(e))
        })?;

        let members = matching_globs(dir, manifest.workspace_members, false).await?;

        Ok(stream! {
            for path in members {
                let manifest = fs::read_to_string(path.join(MANIFEST_FILE_NAME))
                    .await
                    .map_err(errors::WorkspaceMembersError::ManifestMissing)?;
                let manifest = toml::from_str::<Manifest>(&manifest).map_err(|e| {
                    errors::WorkspaceMembersError::ManifestDeser(path.clone(), Box::new(e))
                })?;

                yield Ok((path, manifest));
            }
        })
    }
}

/// Gets all matching paths in a directory
pub async fn matching_globs<P: AsRef<Path>>(
    dir: P,
    members: Vec<globset::Glob>,
    relative: bool,
) -> Result<HashSet<PathBuf>, errors::MatchingGlobsError> {
    let mut positive_globset = globset::GlobSetBuilder::new();
    let mut negative_globset = globset::GlobSetBuilder::new();

    for pattern in members {
        match pattern.glob().strip_prefix('!') {
            Some(pattern) => negative_globset.add(globset::Glob::new(pattern)?),
            None => positive_globset.add(pattern),
        };
    }

    let positive_globset = positive_globset.build()?;
    let negative_globset = negative_globset.build()?;

    let mut read_dirs = vec![fs::read_dir(dir.as_ref().to_path_buf())];
    let mut paths = HashSet::new();

    while let Some(read_dir) = read_dirs.pop() {
        let mut read_dir = read_dir.await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            if entry.file_type().await?.is_dir() {
                read_dirs.push(fs::read_dir(path));
                continue;
            }

            let relative_path = path.strip_prefix(dir.as_ref()).unwrap();

            if positive_globset.is_match(relative_path) && !negative_globset.is_match(relative_path)
            {
                paths.insert(if relative {
                    relative_path.to_path_buf()
                } else {
                    path.to_path_buf()
                });
            }
        }
    }

    Ok(paths)
}

/// Refreshes the sources asynchronously
pub async fn refresh_sources<I: Iterator<Item = PackageSources>>(
    project: &Project,
    sources: I,
    refreshed_sources: &mut HashSet<PackageSources>,
) -> Result<(), Box<source::errors::RefreshError>> {
    try_join_all(sources.map(|source| {
        let needs_refresh = refreshed_sources.insert(source.clone());
        async move {
            if needs_refresh {
                source.refresh(project).await.map_err(Box::new)
            } else {
                Ok(())
            }
        }
    }))
    .await
    .map(|_| ())
}

/// Errors that can occur when using the pesde library
pub mod errors {
    use std::path::PathBuf;
    use thiserror::Error;

    /// Errors that can occur when reading the manifest file
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum ManifestReadError {
        /// An IO error occurred
        #[error("io error reading manifest file")]
        Io(#[from] std::io::Error),

        /// An error occurred while deserializing the manifest file
        #[error("error deserializing manifest file")]
        Serde(#[from] toml::de::Error),
    }

    /// Errors that can occur when reading the lockfile
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum LockfileReadError {
        /// An IO error occurred
        #[error("io error reading lockfile")]
        Io(#[from] std::io::Error),

        /// An error occurred while deserializing the lockfile
        #[error("error deserializing lockfile")]
        Serde(#[from] toml::de::Error),
    }

    /// Errors that can occur when writing the lockfile
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum LockfileWriteError {
        /// An IO error occurred
        #[error("io error writing lockfile")]
        Io(#[from] std::io::Error),

        /// An error occurred while serializing the lockfile
        #[error("error serializing lockfile")]
        Serde(#[from] toml::ser::Error),
    }

    /// Errors that can occur when finding workspace members
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum WorkspaceMembersError {
        /// The manifest file could not be found
        #[error("missing manifest file")]
        ManifestMissing(#[source] std::io::Error),

        /// An error occurred deserializing the manifest file
        #[error("error deserializing manifest file at {0}")]
        ManifestDeser(PathBuf, #[source] Box<toml::de::Error>),

        /// An error occurred interacting with the filesystem
        #[error("error interacting with the filesystem")]
        Io(#[from] std::io::Error),

        /// An error occurred while globbing
        #[error("error globbing")]
        Globbing(#[from] MatchingGlobsError),
    }

    /// Errors that can occur when finding matching globs
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum MatchingGlobsError {
        /// An error occurred interacting with the filesystem
        #[error("error interacting with the filesystem")]
        Io(#[from] std::io::Error),

        /// An error occurred while globbing
        #[error("error globbing")]
        Globbing(#[from] globset::Error),
    }
}
