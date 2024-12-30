use crate::{
    manifest::{
        target::{Target, TargetKind},
        Manifest,
    },
    names::PackageNames,
    reporters::DownloadProgressReporter,
    source::{
        fs::{store_in_cas, FSEntry, PackageFS},
        git::{pkg_ref::GitPackageRef, specifier::GitDependencySpecifier},
        git_index::{read_file, GitBasedSource},
        specifiers::DependencySpecifiers,
        traits::{DownloadOptions, PackageRef, RefreshOptions, ResolveOptions},
        PackageSource, ResolveResult, VersionId, IGNORED_DIRS, IGNORED_FILES,
    },
    util::hash,
    Project, DEFAULT_INDEX_NAME, LOCKFILE_FILE_NAME, MANIFEST_FILE_NAME,
};
use fs_err::tokio as fs;
use futures::future::try_join_all;
use gix::{bstr::BStr, traverse::tree::Recorder, ObjectId, Url};
use relative_path::RelativePathBuf;
use std::{collections::BTreeMap, fmt::Debug, hash::Hash, path::PathBuf, sync::Arc};
use tokio::{sync::Mutex, task::spawn_blocking};
use tracing::instrument;

/// The Git package reference
pub mod pkg_ref;
/// The Git dependency specifier
pub mod specifier;

/// The Git package source
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct GitPackageSource {
    repo_url: Url,
}

impl GitBasedSource for GitPackageSource {
    fn path(&self, project: &Project) -> PathBuf {
        project
            .data_dir()
            .join("git_repos")
            .join(hash(self.as_bytes()))
    }

    fn repo_url(&self) -> &Url {
        &self.repo_url
    }
}

impl GitPackageSource {
    /// Creates a new Git package source
    pub fn new(repo_url: Url) -> Self {
        Self { repo_url }
    }

    fn as_bytes(&self) -> Vec<u8> {
        self.repo_url.to_bstring().to_vec()
    }
}

impl PackageSource for GitPackageSource {
    type Specifier = GitDependencySpecifier;
    type Ref = GitPackageRef;
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
        let ResolveOptions { project, .. } = options;

        let repo = gix::open(self.path(project))
            .map_err(|e| errors::ResolveError::OpenRepo(Box::new(self.repo_url.clone()), e))?;
        let rev = repo
            .rev_parse_single(BStr::new(&specifier.rev))
            .map_err(|e| {
                errors::ResolveError::ParseRev(
                    specifier.rev.clone(),
                    Box::new(self.repo_url.clone()),
                    e,
                )
            })?;

        // TODO: possibly use the search algorithm from src/main.rs to find the workspace root

        let root_tree = rev
            .object()
            .map_err(|e| {
                errors::ResolveError::ParseRevToObject(Box::new(self.repo_url.clone()), e)
            })?
            .peel_to_tree()
            .map_err(|e| {
                errors::ResolveError::ParseObjectToTree(Box::new(self.repo_url.clone()), e)
            })?;

        let tree = if let Some(path) = &specifier.path {
            root_tree
                .lookup_entry_by_path(path.as_str())
                .map_err(|e| {
                    errors::ResolveError::ReadTreeEntry(
                        Box::new(self.repo_url.clone()),
                        path.clone(),
                        e,
                    )
                })?
                .ok_or_else(|| {
                    errors::ResolveError::NoEntryAtPath(
                        Box::new(self.repo_url.clone()),
                        path.clone(),
                    )
                })?
                .object()
                .map_err(|e| {
                    errors::ResolveError::ParseEntryToObject(Box::new(self.repo_url.clone()), e)
                })?
                .peel_to_tree()
                .map_err(|e| {
                    errors::ResolveError::ParseObjectToTree(Box::new(self.repo_url.clone()), e)
                })?
        } else {
            root_tree.clone()
        };

        let manifest = match read_file(&tree, [MANIFEST_FILE_NAME])
            .map_err(|e| errors::ResolveError::ReadManifest(Box::new(self.repo_url.clone()), e))?
        {
            Some(m) => match toml::from_str::<Manifest>(&m) {
                Ok(m) => Some(m),
                Err(e) => {
                    return Err(errors::ResolveError::DeserManifest(
                        Box::new(self.repo_url.clone()),
                        e,
                    ))
                }
            },
            None => None,
        };

        let (name, version_id, dependencies) = match manifest {
            Some(manifest) => {
                let dependencies = manifest
                    .all_dependencies()
                    .map_err(|e| {
                        errors::ResolveError::CollectDependencies(
                            Box::new(self.repo_url.clone()),
                            e,
                        )
                    })?
                    .into_iter()
                    .map(|(alias, (mut spec, ty))| {
                        match &mut spec {
                            DependencySpecifiers::Pesde(specifier) => {
                                let index_name = specifier
                                    .index
                                    .as_deref()
                                    .unwrap_or(DEFAULT_INDEX_NAME)
                                    .to_string();
                                specifier.index = Some(
                                    manifest
                                        .indices
                                        .get(&index_name)
                                        .ok_or_else(|| {
                                            errors::ResolveError::PesdeIndexNotFound(
                                                index_name.clone(),
                                                Box::new(self.repo_url.clone()),
                                            )
                                        })?
                                        .to_string(),
                                );
                            }
                            #[cfg(feature = "wally-compat")]
                            DependencySpecifiers::Wally(specifier) => {
                                let index_name = specifier
                                    .index
                                    .as_deref()
                                    .unwrap_or(DEFAULT_INDEX_NAME)
                                    .to_string();
                                specifier.index = Some(
                                    manifest
                                        .wally_indices
                                        .get(&index_name)
                                        .ok_or_else(|| {
                                            errors::ResolveError::WallyIndexNotFound(
                                                index_name.clone(),
                                                Box::new(self.repo_url.clone()),
                                            )
                                        })?
                                        .to_string(),
                                );
                            }
                            DependencySpecifiers::Git(_) => {}
                            DependencySpecifiers::Workspace(specifier) => {
                                let lockfile = read_file(&root_tree, [LOCKFILE_FILE_NAME])
                                    .map_err(|e| {
                                        errors::ResolveError::ReadLockfile(
                                            Box::new(self.repo_url.clone()),
                                            e,
                                        )
                                    })?;

                                let lockfile = match lockfile {
                                    Some(l) => match toml::from_str::<crate::Lockfile>(&l) {
                                        Ok(l) => l,
                                        Err(e) => {
                                            return Err(errors::ResolveError::DeserLockfile(
                                                Box::new(self.repo_url.clone()),
                                                e,
                                            ))
                                        }
                                    },
                                    None => {
                                        return Err(errors::ResolveError::NoLockfile(Box::new(
                                            self.repo_url.clone(),
                                        )))
                                    }
                                };

                                let target = specifier.target.unwrap_or(manifest.target.kind());

                                let path = lockfile
                                    .workspace
                                    .get(&specifier.name)
                                    .and_then(|targets| targets.get(&target))
                                    .ok_or_else(|| {
                                        errors::ResolveError::NoPathForWorkspaceMember(
                                            specifier.name.to_string(),
                                            target,
                                            Box::new(self.repo_url.clone()),
                                        )
                                    })?
                                    .clone();

                                spec = DependencySpecifiers::Git(GitDependencySpecifier {
                                    repo: self.repo_url.clone(),
                                    rev: rev.to_string(),
                                    path: Some(path),
                                })
                            }
                            DependencySpecifiers::Path(_) => {
                                return Err(errors::ResolveError::Path(Box::new(
                                    self.repo_url.clone(),
                                )))
                            }
                        }

                        Ok((alias, (spec, ty)))
                    })
                    .collect::<Result<_, errors::ResolveError>>()?;
                let name = PackageNames::Pesde(manifest.name);
                let version_id = VersionId(manifest.version, manifest.target.kind());

                (name, version_id, dependencies)
            }

            #[cfg(feature = "wally-compat")]
            None => {
                match read_file(
                    &tree,
                    [crate::source::wally::compat_util::WALLY_MANIFEST_FILE_NAME],
                )
                .map_err(|e| {
                    errors::ResolveError::ReadManifest(Box::new(self.repo_url.clone()), e)
                })? {
                    Some(m) => {
                        match toml::from_str::<crate::source::wally::manifest::WallyManifest>(&m) {
                            Ok(manifest) => {
                                let dependencies = manifest.all_dependencies().map_err(|e| {
                                    errors::ResolveError::CollectDependencies(
                                        Box::new(self.repo_url.clone()),
                                        e,
                                    )
                                })?;
                                let name = PackageNames::Wally(manifest.package.name);
                                let version_id = VersionId(
                                    manifest.package.version,
                                    match manifest.package.realm {
                                        crate::source::wally::manifest::Realm::Server => {
                                            TargetKind::RobloxServer
                                        }
                                        _ => TargetKind::Roblox,
                                    },
                                );

                                (name, version_id, dependencies)
                            }
                            Err(e) => {
                                return Err(errors::ResolveError::DeserManifest(
                                    Box::new(self.repo_url.clone()),
                                    e,
                                ))
                            }
                        }
                    }
                    None => {
                        return Err(errors::ResolveError::NoManifest(Box::new(
                            self.repo_url.clone(),
                        )))
                    }
                }
            }
            #[cfg(not(feature = "wally-compat"))]
            None => {
                return Err(errors::ResolveError::NoManifest(Box::new(
                    self.repo_url.clone(),
                )))
            }
        };

        let new_structure = matches!(name, PackageNames::Pesde(_));

        Ok((
            name,
            BTreeMap::from([(
                version_id,
                GitPackageRef {
                    repo: self.repo_url.clone(),
                    tree_id: tree.id.to_string(),
                    new_structure,
                    dependencies,
                },
            )]),
        ))
    }

    #[instrument(skip_all, level = "debug")]
    async fn download<R: DownloadProgressReporter>(
        &self,
        pkg_ref: &Self::Ref,
        options: &DownloadOptions<R>,
    ) -> Result<(PackageFS, Target), Self::DownloadError> {
        let DownloadOptions { project, .. } = options;

        let index_file = project
            .cas_dir()
            .join("git_index")
            .join(hash(self.as_bytes()))
            .join(&pkg_ref.tree_id);

        match fs::read_to_string(&index_file).await {
            Ok(s) => {
                tracing::debug!(
                    "using cached index file for package {}#{}",
                    pkg_ref.repo,
                    pkg_ref.tree_id
                );

                let fs = toml::from_str::<PackageFS>(&s).map_err(|e| {
                    errors::DownloadError::DeserializeFile(Box::new(self.repo_url.clone()), e)
                })?;

                let manifest = match &fs {
                    PackageFS::CAS(entries) => {
                        match entries.get(&RelativePathBuf::from(MANIFEST_FILE_NAME)) {
                            Some(FSEntry::File(hash)) => match fs
                                .read_file(hash, project.cas_dir())
                                .await
                                .map(|m| toml::de::from_str::<Manifest>(&m))
                            {
                                Some(Ok(m)) => Some(m),
                                Some(Err(e)) => {
                                    return Err(errors::DownloadError::DeserializeFile(
                                        Box::new(self.repo_url.clone()),
                                        e,
                                    ))
                                }
                                None => None,
                            },
                            _ => None,
                        }
                    }
                    _ => unreachable!("the package fs should be CAS"),
                };

                let target = match manifest {
                    Some(manifest) => manifest.target,
                    #[cfg(feature = "wally-compat")]
                    None if !pkg_ref.new_structure => {
                        let tempdir = tempfile::tempdir()?;
                        fs.write_to(tempdir.path(), project.cas_dir(), false)
                            .await?;

                        crate::source::wally::compat_util::get_target(project, &tempdir).await?
                    }
                    None => {
                        return Err(errors::DownloadError::NoManifest(Box::new(
                            self.repo_url.clone(),
                        )))
                    }
                };

                return Ok((fs, target));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(errors::DownloadError::Io(e)),
        }

        let repo = gix::open(self.path(project))
            .map_err(|e| errors::DownloadError::OpenRepo(Box::new(self.repo_url.clone()), e))?
            .into_sync();
        let repo_url = self.repo_url.clone();
        let tree_id = pkg_ref.tree_id.clone();

        let (repo, records) = spawn_blocking(move || {
            let repo = repo.to_thread_local();

            let mut recorder = Recorder::default();

            {
                let object_id = match tree_id.parse::<ObjectId>() {
                    Ok(oid) => oid,
                    Err(e) => {
                        return Err(errors::DownloadError::ParseTreeId(Box::new(repo_url), e))
                    }
                };
                let object = match repo.find_object(object_id) {
                    Ok(object) => object,
                    Err(e) => {
                        return Err(errors::DownloadError::ParseOidToObject(
                            object_id,
                            Box::new(repo_url),
                            e,
                        ))
                    }
                };

                let tree = match object.peel_to_tree() {
                    Ok(tree) => tree,
                    Err(e) => {
                        return Err(errors::DownloadError::ParseObjectToTree(
                            Box::new(repo_url),
                            e,
                        ))
                    }
                };

                if let Err(e) = tree.traverse().breadthfirst(&mut recorder) {
                    return Err(errors::DownloadError::TraverseTree(Box::new(repo_url), e));
                }
            }

            Ok::<_, errors::DownloadError>((repo.into_sync(), recorder.records))
        })
        .await
        .unwrap()?;

        let repo = repo.to_thread_local();

        let records = records
            .into_iter()
            .map(|entry| {
                let object = repo.find_object(entry.oid).map_err(|e| {
                    errors::DownloadError::ParseOidToObject(
                        entry.oid,
                        Box::new(self.repo_url.clone()),
                        e,
                    )
                })?;

                Ok::<_, errors::DownloadError>((
                    RelativePathBuf::from(entry.filepath.to_string()),
                    if matches!(object.kind, gix::object::Kind::Tree) {
                        None
                    } else {
                        Some(object.data.clone())
                    },
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let manifest = Arc::new(Mutex::new(None::<Vec<u8>>));
        let entries = try_join_all(
            records
                .into_iter()
                .filter(|(path, contents)| {
                    let name = path.file_name().unwrap_or("");
                    if contents.is_none() {
                        return !IGNORED_DIRS.contains(&name);
                    }

                    if IGNORED_FILES.contains(&name) {
                        return false;
                    }

                    if pkg_ref.use_new_structure() && name == "default.project.json" {
                        tracing::debug!(
                            "removing default.project.json from {}#{} at {path} - using new structure",
                            pkg_ref.repo,
                            pkg_ref.tree_id
                        );
                        return false;
                    }

                    true
                })
                .map(|(path, contents)| {
                    let manifest = manifest.clone();
                    async move {
                        let Some(contents) = contents else {
                            return Ok::<_, errors::DownloadError>((path, FSEntry::Directory));
                        };

                        let hash =
                            store_in_cas(project.cas_dir(), contents.as_slice(), |_| async { Ok(()) })
                                .await?;

                        if path == MANIFEST_FILE_NAME {
                            manifest.lock().await.replace(contents);
                        }

                        Ok((path, FSEntry::File(hash)))
                    }
                }),
        )
            .await?
            .into_iter()
            .collect::<BTreeMap<_, _>>();

        let manifest = match Arc::into_inner(manifest).unwrap().into_inner() {
            Some(data) => match String::from_utf8(data.to_vec()) {
                Ok(s) => match toml::from_str::<Manifest>(&s) {
                    Ok(m) => Some(m),
                    Err(e) => {
                        return Err(errors::DownloadError::DeserializeFile(
                            Box::new(self.repo_url.clone()),
                            e,
                        ))
                    }
                },
                Err(e) => return Err(errors::DownloadError::ParseManifest(e)),
            },
            None => None,
        };

        let fs = PackageFS::CAS(entries);

        let target = match manifest {
            Some(manifest) => manifest.target,
            #[cfg(feature = "wally-compat")]
            None if !pkg_ref.new_structure => {
                let tempdir = tempfile::tempdir()?;
                fs.write_to(tempdir.path(), project.cas_dir(), false)
                    .await?;

                crate::source::wally::compat_util::get_target(project, &tempdir).await?
            }
            None => {
                return Err(errors::DownloadError::NoManifest(Box::new(
                    self.repo_url.clone(),
                )))
            }
        };

        if let Some(parent) = index_file.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::write(
            &index_file,
            toml::to_string(&fs).map_err(|e| {
                errors::DownloadError::SerializeIndex(Box::new(self.repo_url.clone()), e)
            })?,
        )
        .await
        .map_err(errors::DownloadError::Io)?;

        Ok((fs, target))
    }
}

/// Errors that can occur when interacting with the Git package source
pub mod errors {
    use crate::manifest::target::TargetKind;
    use gix::ObjectId;
    use relative_path::RelativePathBuf;
    use thiserror::Error;

    /// Errors that can occur when resolving a package from a Git package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum ResolveError {
        /// An error occurred opening the Git repository
        #[error("error opening Git repository for url {0}")]
        OpenRepo(Box<gix::Url>, #[source] gix::open::Error),

        /// An error occurred parsing rev
        #[error("error parsing rev {0} for repository {1}")]
        ParseRev(
            String,
            Box<gix::Url>,
            #[source] gix::revision::spec::parse::single::Error,
        ),

        /// An error occurred parsing rev to object
        #[error("error parsing rev to object for repository {0}")]
        ParseRevToObject(Box<gix::Url>, #[source] gix::object::find::existing::Error),

        /// An error occurred parsing object to tree
        #[error("error parsing object to tree for repository {0}")]
        ParseObjectToTree(Box<gix::Url>, #[source] gix::object::peel::to_kind::Error),

        /// An error occurred reading the manifest
        #[error("error reading manifest of repository {0}")]
        ReadManifest(
            Box<gix::Url>,
            #[source] crate::source::git_index::errors::ReadFile,
        ),

        /// An error occurred collecting all manifest dependencies
        #[error("error collecting all manifest dependencies for repository {0}")]
        CollectDependencies(
            Box<gix::Url>,
            #[source] crate::manifest::errors::AllDependenciesError,
        ),

        /// An error occurred deserializing a manifest
        #[error("error deserializing manifest for repository {0}")]
        DeserManifest(Box<gix::Url>, #[source] toml::de::Error),

        /// No manifest was found
        #[error("no manifest found in repository {0}")]
        NoManifest(Box<gix::Url>),

        /// A pesde index was not found in the manifest
        #[error("pesde index {0} not found in manifest for repository {1}")]
        PesdeIndexNotFound(String, Box<gix::Url>),

        /// A Wally index was not found in the manifest
        #[error("wally index {0} not found in manifest for repository {1}")]
        WallyIndexNotFound(String, Box<gix::Url>),

        /// An error occurred reading a tree entry
        #[error("error reading tree entry for repository {0} at {1}")]
        ReadTreeEntry(
            Box<gix::Url>,
            RelativePathBuf,
            #[source] gix::object::find::existing::Error,
        ),

        /// No entry was found at the specified path
        #[error("no entry found at path {1} in repository {0}")]
        NoEntryAtPath(Box<gix::Url>, RelativePathBuf),

        /// An error occurred parsing an entry to object
        #[error("error parsing an entry to object for repository {0}")]
        ParseEntryToObject(Box<gix::Url>, #[source] gix::object::find::existing::Error),

        /// An error occurred reading the lockfile
        #[error("error reading lockfile for repository {0}")]
        ReadLockfile(
            Box<gix::Url>,
            #[source] crate::source::git_index::errors::ReadFile,
        ),

        /// An error occurred while deserializing the lockfile
        #[error("error deserializing lockfile for repository {0}")]
        DeserLockfile(Box<gix::Url>, #[source] toml::de::Error),

        /// The repository is missing a lockfile
        #[error("no lockfile found in repository {0}")]
        NoLockfile(Box<gix::Url>),

        /// No path for a workspace member was found in the lockfile
        #[error("no path found for workspace member {0} {1} in lockfile for repository {2}")]
        NoPathForWorkspaceMember(String, TargetKind, Box<gix::Url>),

        /// The package depends on a path package
        #[error("the package {0} depends on a path package")]
        Path(Box<gix::Url>),
    }

    /// Errors that can occur when downloading a package from a Git package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum DownloadError {
        /// An error occurred deserializing a file
        #[error("error deserializing file in repository {0}")]
        DeserializeFile(Box<gix::Url>, #[source] toml::de::Error),

        /// An error occurred interacting with the file system
        #[error("error interacting with the file system")]
        Io(#[from] std::io::Error),

        /// An error occurred while searching for a Wally lib export
        #[cfg(feature = "wally-compat")]
        #[error("error searching for Wally lib export")]
        FindLibPath(#[from] crate::source::wally::compat_util::errors::FindLibPathError),

        /// No manifest was found
        #[error("no manifest found in repository {0}")]
        NoManifest(Box<gix::Url>),

        /// An error occurred opening the Git repository
        #[error("error opening Git repository for url {0}")]
        OpenRepo(Box<gix::Url>, #[source] gix::open::Error),

        /// An error occurred while traversing the tree
        #[error("error traversing tree for repository {0}")]
        TraverseTree(
            Box<gix::Url>,
            #[source] gix::traverse::tree::breadthfirst::Error,
        ),

        /// An error occurred parsing an object id to object
        #[error("error parsing object id {0} to object for repository {1}")]
        ParseOidToObject(
            ObjectId,
            Box<gix::Url>,
            #[source] gix::object::find::existing::Error,
        ),

        /// An error occurred parsing object to tree
        #[error("error parsing object to tree for repository {0}")]
        ParseObjectToTree(Box<gix::Url>, #[source] gix::object::peel::to_kind::Error),

        /// An error occurred parsing the pesde manifest to UTF-8
        #[error("error parsing the manifest for repository {0} to UTF-8")]
        ParseManifest(#[source] std::string::FromUtf8Error),

        /// An error occurred while serializing the index file
        #[error("error serializing the index file for repository {0}")]
        SerializeIndex(Box<gix::Url>, #[source] toml::ser::Error),

        /// An error occurred while parsing tree_id to ObjectId
        #[error("error parsing tree_id to ObjectId for repository {0}")]
        ParseTreeId(Box<gix::Url>, #[source] gix::hash::decode::Error),
    }
}
