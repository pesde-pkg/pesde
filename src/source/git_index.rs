use gix::remote::Direction;

use crate::{util::authenticate_conn, Project};

/// A trait for sources that are based on Git repositories
pub trait GitBasedSource {
    /// The path to the index
    fn path(&self, project: &Project) -> std::path::PathBuf;

    /// The URL of the repository
    fn repo_url(&self) -> &gix::Url;

    /// Gets the tree of the repository
    fn tree<'a>(&'a self, repo: &'a gix::Repository) -> Result<gix::Tree<'a>, errors::TreeError> {
        // this is a bare repo, so this is the actual path
        let path = repo.path().to_path_buf();

        let remote = match repo.find_default_remote(Direction::Fetch) {
            Some(Ok(remote)) => remote,
            Some(Err(e)) => return Err(errors::TreeError::GetDefaultRemote(path, Box::new(e))),
            None => {
                return Err(errors::TreeError::NoDefaultRemote(path));
            }
        };

        let refspec = match remote.refspecs(Direction::Fetch).first() {
            Some(head) => head,
            None => return Err(errors::TreeError::NoRefSpecs(path)),
        };

        let spec_ref = refspec.to_ref();
        let local_ref = match spec_ref.local() {
            Some(local) => local
                .to_string()
                .replace('*', repo.branch_names().first().unwrap_or(&"main")),
            None => return Err(errors::TreeError::NoLocalRefSpec(path)),
        };

        let reference = match repo.find_reference(&local_ref) {
            Ok(reference) => reference,
            Err(e) => return Err(errors::TreeError::NoReference(local_ref.to_string(), e)),
        };

        let reference_name = reference.name().as_bstr().to_string();
        let id = match reference.into_fully_peeled_id() {
            Ok(id) => id,
            Err(e) => return Err(errors::TreeError::CannotPeel(reference_name, e)),
        };

        let id_str = id.to_string();
        let object = match id.object() {
            Ok(object) => object,
            Err(e) => return Err(errors::TreeError::CannotConvertToObject(id_str, e)),
        };

        match object.peel_to_tree() {
            Ok(tree) => Ok(tree),
            Err(e) => Err(errors::TreeError::CannotPeelToTree(id_str, e)),
        }
    }

    /// Reads a file from the repository
    fn read_file<I: IntoIterator<Item = P> + Clone, P: ToString + PartialEq<gix::bstr::BStr>>(
        &self,
        file_path: I,
        project: &Project,
        tree: Option<gix::Tree>,
    ) -> Result<Option<String>, errors::ReadFile> {
        let path = self.path(project);

        let repo = match gix::open(&path) {
            Ok(repo) => repo,
            Err(e) => return Err(errors::ReadFile::Open(path, Box::new(e))),
        };

        let tree = match tree.map_or_else(|| self.tree(&repo), Ok) {
            Ok(tree) => tree,
            Err(e) => return Err(errors::ReadFile::Tree(path, Box::new(e))),
        };

        let file_path_str = file_path
            .clone()
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(std::path::MAIN_SEPARATOR_STR);

        let entry = match tree.lookup_entry(file_path) {
            Ok(Some(entry)) => entry,
            Ok(None) => return Ok(None),
            Err(e) => return Err(errors::ReadFile::Lookup(file_path_str, e)),
        };

        let object = match entry.object() {
            Ok(object) => object,
            Err(e) => return Err(errors::ReadFile::Lookup(file_path_str, e)),
        };

        let blob = object.into_blob();
        let string = String::from_utf8(blob.data.clone())
            .map_err(|e| errors::ReadFile::Utf8(file_path_str, e))?;

        Ok(Some(string))
    }

    /// Refreshes the repository
    fn refresh(&self, project: &Project) -> Result<(), errors::RefreshError> {
        let path = self.path(project);
        if path.exists() {
            let repo = match gix::open(&path) {
                Ok(repo) => repo,
                Err(e) => return Err(errors::RefreshError::Open(path, Box::new(e))),
            };
            let remote = match repo.find_default_remote(Direction::Fetch) {
                Some(Ok(remote)) => remote,
                Some(Err(e)) => {
                    return Err(errors::RefreshError::GetDefaultRemote(path, Box::new(e)))
                }
                None => {
                    return Err(errors::RefreshError::NoDefaultRemote(path));
                }
            };

            let mut connection = remote.connect(Direction::Fetch).map_err(|e| {
                errors::RefreshError::Connect(self.repo_url().to_string(), Box::new(e))
            })?;

            authenticate_conn(&mut connection, &project.auth_config);

            connection
                .prepare_fetch(gix::progress::Discard, Default::default())
                .map_err(|e| {
                    errors::RefreshError::PrepareFetch(self.repo_url().to_string(), Box::new(e))
                })?
                .receive(gix::progress::Discard, &false.into())
                .map_err(|e| {
                    errors::RefreshError::Read(self.repo_url().to_string(), Box::new(e))
                })?;

            return Ok(());
        }

        std::fs::create_dir_all(&path)?;

        let auth_config = project.auth_config.clone();

        gix::prepare_clone_bare(self.repo_url().clone(), &path)
            .map_err(|e| errors::RefreshError::Clone(self.repo_url().to_string(), Box::new(e)))?
            .configure_connection(move |c| {
                authenticate_conn(c, &auth_config);
                Ok(())
            })
            .fetch_only(gix::progress::Discard, &false.into())
            .map_err(|e| errors::RefreshError::Fetch(self.repo_url().to_string(), Box::new(e)))?;

        Ok(())
    }
}

/// Errors that can occur when interacting with a git-based package source
pub mod errors {
    use std::path::PathBuf;

    use thiserror::Error;

    /// Errors that can occur when refreshing a git-based package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum RefreshError {
        /// Error interacting with the filesystem
        #[error("error interacting with the filesystem")]
        Io(#[from] std::io::Error),

        /// Error opening the repository
        #[error("error opening repository at {0}")]
        Open(PathBuf, #[source] Box<gix::open::Error>),

        /// No default remote found in repository
        #[error("no default remote found in repository at {0}")]
        NoDefaultRemote(PathBuf),

        /// Error getting default remote from repository
        #[error("error getting default remote from repository at {0}")]
        GetDefaultRemote(PathBuf, #[source] Box<gix::remote::find::existing::Error>),

        /// Error connecting to remote repository
        #[error("error connecting to remote repository at {0}")]
        Connect(String, #[source] Box<gix::remote::connect::Error>),

        /// Error preparing fetch from remote repository
        #[error("error preparing fetch from remote repository at {0}")]
        PrepareFetch(String, #[source] Box<gix::remote::fetch::prepare::Error>),

        /// Error reading from remote repository
        #[error("error reading from remote repository at {0}")]
        Read(String, #[source] Box<gix::remote::fetch::Error>),

        /// Error cloning repository
        #[error("error cloning repository from {0}")]
        Clone(String, #[source] Box<gix::clone::Error>),

        /// Error fetching repository
        #[error("error fetching repository from {0}")]
        Fetch(String, #[source] Box<gix::clone::fetch::Error>),
    }

    /// Errors that can occur when reading a git-based package source's tree
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum TreeError {
        /// Error interacting with the filesystem
        #[error("error interacting with the filesystem")]
        Io(#[from] std::io::Error),

        /// No default remote found in repository
        #[error("no default remote found in repository at {0}")]
        NoDefaultRemote(PathBuf),

        /// Error getting default remote from repository
        #[error("error getting default remote from repository at {0}")]
        GetDefaultRemote(PathBuf, #[source] Box<gix::remote::find::existing::Error>),

        /// Error getting refspec from remote repository
        #[error("no refspecs found in repository at {0}")]
        NoRefSpecs(PathBuf),

        /// Error getting local refspec from remote repository
        #[error("no local refspec found in repository at {0}")]
        NoLocalRefSpec(PathBuf),

        /// Error finding reference in repository
        #[error("no reference found for local refspec {0}")]
        NoReference(String, #[source] gix::reference::find::existing::Error),

        /// Error peeling reference in repository
        #[error("cannot peel reference {0}")]
        CannotPeel(String, #[source] gix::reference::peel::Error),

        /// Error converting id to object in repository
        #[error("error converting id {0} to object")]
        CannotConvertToObject(String, #[source] gix::object::find::existing::Error),

        /// Error peeling object to tree in repository
        #[error("error peeling object {0} to tree")]
        CannotPeelToTree(String, #[source] gix::object::peel::to_kind::Error),
    }

    /// Errors that can occur when reading a file from a git-based package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum ReadFile {
        /// Error opening the repository
        #[error("error opening repository at {0}")]
        Open(PathBuf, #[source] Box<gix::open::Error>),

        /// Error reading tree from repository
        #[error("error getting tree from repository at {0}")]
        Tree(PathBuf, #[source] Box<TreeError>),

        /// Error looking up entry in tree
        #[error("error looking up entry {0} in tree")]
        Lookup(String, #[source] gix::object::find::existing::Error),

        /// Error reading file as utf8
        #[error("error parsing file for {0} as utf8")]
        Utf8(String, #[source] std::string::FromUtf8Error),
    }
}
