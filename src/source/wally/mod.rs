use crate::{
    manifest::target::{Target, TargetKind},
    names::PackageNames,
    reporters::DownloadProgressReporter,
    source::{
        fs::{store_in_cas, FSEntry, PackageFS},
        git_index::{read_file, root_tree, GitBasedSource},
        traits::{DownloadOptions, PackageSource, RefreshOptions, ResolveOptions},
        version_id::VersionId,
        wally::{
            compat_util::get_target,
            manifest::{Realm, WallyManifest},
            pkg_ref::WallyPackageRef,
        },
        PackageSources, ResolveResult, IGNORED_DIRS, IGNORED_FILES,
    },
    util::hash,
    Project,
};
use fs_err::tokio as fs;
use futures::StreamExt;
use gix::Url;
use relative_path::RelativePathBuf;
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tempfile::tempdir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
    task::spawn_blocking,
};
use tokio_util::{compat::FuturesAsyncReadCompatExt, io::StreamReader};
use tracing::instrument;

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
            .data_dir()
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
    #[instrument(skip_all, ret(level = "trace"), level = "debug")]
    pub async fn config(&self, project: &Project) -> Result<WallyIndexConfig, errors::ConfigError> {
        let repo_url = self.repo_url.clone();
        let path = self.path(project);

        spawn_blocking(move || {
            let repo = gix::open(&path).map_err(Box::new)?;
            let tree = root_tree(&repo).map_err(Box::new)?;
            let file = read_file(&tree, ["config.json"]).map_err(Box::new)?;

            match file {
                Some(s) => serde_json::from_str(&s).map_err(Into::into),
                None => Err(errors::ConfigError::Missing(Box::new(repo_url))),
            }
        })
        .await
        .unwrap()
    }
}

impl PackageSource for WallyPackageSource {
    type Specifier = specifier::WallyDependencySpecifier;
    type Ref = WallyPackageRef;
    type RefreshError = crate::source::git_index::errors::RefreshError;
    type ResolveError = errors::ResolveError;
    type DownloadError = errors::DownloadError;

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
            refreshed_sources,
            ..
        } = options;

        let repo = gix::open(self.path(project)).map_err(Box::new)?;
        let tree = root_tree(&repo).map_err(Box::new)?;
        let (scope, name) = specifier.name.as_str();
        let string = match read_file(&tree, [scope, name]) {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::debug!(
                    "{} not found in wally registry. searching in backup registries",
                    specifier.name
                );

                let config = self.config(project).await.map_err(Box::new)?;
                for registry in config.fallback_registries {
                    let source = WallyPackageSource::new(registry);
                    match refreshed_sources
                        .refresh(
                            &PackageSources::Wally(source.clone()),
                            &RefreshOptions {
                                project: project.clone(),
                            },
                        )
                        .await
                    {
                        Ok(()) => {}
                        Err(super::errors::RefreshError::Wally(e)) => {
                            return Err(Self::ResolveError::Refresh(Box::new(e)));
                        }
                        Err(e) => unreachable!("unexpected error: {e:?}"),
                    }

                    match Box::pin(source.resolve(specifier, options)).await {
                        Ok((name, results)) => {
                            tracing::debug!("found {name} in backup registry {}", source.repo_url);
                            return Ok((name, results));
                        }
                        Err(errors::ResolveError::NotFound(_)) => {
                            continue;
                        }
                        Err(e) => {
                            return Err(e);
                        }
                    }
                }

                return Err(Self::ResolveError::NotFound(specifier.name.to_string()));
            }
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

        tracing::debug!("{} has {} possible entries", specifier.name, entries.len());

        Ok((
            PackageNames::Wally(specifier.name.clone()),
            entries
                .into_iter()
                .filter(|manifest| specifier.version.matches(&manifest.package.version))
                .map(|manifest| {
                    Ok((
                        VersionId(
                            manifest.package.version.clone(),
                            match manifest.package.realm {
                                Realm::Server => TargetKind::RobloxServer,
                                _ => TargetKind::Roblox,
                            },
                        ),
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

    #[instrument(skip_all, level = "debug")]
    async fn download<R: DownloadProgressReporter>(
        &self,
        pkg_ref: &Self::Ref,
        options: &DownloadOptions<R>,
    ) -> Result<(PackageFS, Target), Self::DownloadError> {
        let DownloadOptions {
            project,
            reqwest,
            reporter,
        } = options;

        let config = self.config(project).await.map_err(Box::new)?;
        let index_file = project
            .cas_dir()
            .join("wally_index")
            .join(pkg_ref.name.escaped())
            .join(pkg_ref.version.to_string());

        let tempdir = match fs::read_to_string(&index_file).await {
            Ok(s) => {
                tracing::debug!(
                    "using cached index file for package {}@{}",
                    pkg_ref.name,
                    pkg_ref.version
                );

                let tempdir = tempdir()?;
                let fs = toml::from_str::<PackageFS>(&s)?;

                fs.write_to(&tempdir, project.cas_dir(), false).await?;

                return Ok((fs, get_target(project, &tempdir).await?));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => tempdir()?,
            Err(e) => return Err(errors::DownloadError::ReadIndex(e)),
        };

        let (scope, name) = pkg_ref.name.as_str();

        let mut request = reqwest
            .get(format!(
                "{}/v1/package-contents/{scope}/{name}/{}",
                config.api.as_str().trim_end_matches('/'),
                pkg_ref.version
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

        let mut bytes = StreamReader::new(bytes);
        let mut buf = Vec::with_capacity(total_len as usize);
        bytes.read_to_end(&mut buf).await?;

        let mut archive =
            async_zip::tokio::read::seek::ZipFileReader::with_tokio(std::io::Cursor::new(&mut buf))
                .await?;

        let archive_entries = (0..archive.file().entries().len())
            .map(|index| {
                let entry = archive.file().entries().get(index).unwrap();
                let relative_path = RelativePathBuf::from_path(entry.filename().as_str()?).unwrap();
                Ok::<_, errors::DownloadError>((index, entry.dir()?, relative_path))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut entries = BTreeMap::new();

        for (index, is_dir, relative_path) in archive_entries {
            let name = relative_path.file_name().unwrap_or("");
            if if is_dir {
                IGNORED_DIRS.contains(&name)
            } else {
                IGNORED_FILES.contains(&name)
            } {
                continue;
            }

            let path = relative_path.to_path(tempdir.path());

            if is_dir {
                fs::create_dir_all(&path).await?;
                entries.insert(relative_path, FSEntry::Directory);
                continue;
            }

            let entry_reader = archive.reader_without_entry(index).await?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).await?;
            }

            let writer = Arc::new(Mutex::new(fs::File::create(&path).await?));
            let hash = store_in_cas(project.cas_dir(), entry_reader.compat(), |bytes| {
                let writer = writer.clone();
                async move { writer.lock().await.write_all(&bytes).await }
            })
            .await?;

            entries.insert(relative_path, FSEntry::File(hash));
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

        reporter.report_done();

        Ok((fs, get_target(project, &tempdir).await?))
    }
}

/// A Wally index config
#[derive(Debug, Clone, Deserialize)]
pub struct WallyIndexConfig {
    api: url::Url,
    #[serde(default, deserialize_with = "crate::util::deserialize_gix_url_vec")]
    fallback_registries: Vec<Url>,
}

/// Errors that can occur when interacting with a Wally package source
pub mod errors {
    use thiserror::Error;

    use crate::source::git_index::errors::ReadFile;

    /// Errors that can occur when resolving a package from a Wally package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum ResolveError {
        /// Error opening repository
        #[error("error opening repository")]
        Open(#[from] Box<gix::open::Error>),

        /// Error getting tree
        #[error("error getting tree")]
        Tree(#[from] Box<crate::source::git_index::errors::TreeError>),

        /// Package not found in index
        #[error("package {0} not found")]
        NotFound(String),

        /// Error reading file for package
        #[error("error reading file for {0}")]
        Read(String, #[source] Box<ReadFile>),

        /// Error parsing file for package
        #[error("error parsing file for {0}")]
        Parse(String, #[source] serde_json::Error),

        /// Error parsing all dependencies
        #[error("error parsing all dependencies for {0}")]
        AllDependencies(
            String,
            #[source] crate::manifest::errors::AllDependenciesError,
        ),

        /// Error reading config file
        #[error("error reading config file")]
        Config(#[from] Box<ConfigError>),

        /// Error refreshing backup registry source
        #[error("error refreshing backup registry source")]
        Refresh(#[from] Box<crate::source::git_index::errors::RefreshError>),

        /// Error resolving package in backup registries
        #[error("error resolving package in backup registries")]
        BackupResolve(#[from] Box<ResolveError>),
    }

    /// Errors that can occur when reading the config file for a Wally package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum ConfigError {
        /// Error opening repository
        #[error("error opening repository")]
        Open(#[from] Box<gix::open::Error>),

        /// Error getting tree
        #[error("error getting tree")]
        Tree(#[from] Box<crate::source::git_index::errors::TreeError>),

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
        Decompress(#[from] async_zip::error::ZipError),

        /// Error interacting with the filesystem
        #[error("error interacting with the filesystem")]
        Io(#[from] std::io::Error),

        /// Error serializing index file
        #[error("error serializing index file")]
        SerializeIndex(#[from] toml::ser::Error),

        /// Creating the target failed
        #[error("error creating a target")]
        GetTarget(#[from] crate::source::wally::compat_util::errors::GetTargetError),

        /// Error writing index file
        #[error("error writing index file")]
        WriteIndex(#[source] std::io::Error),
    }
}
