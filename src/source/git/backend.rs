//! Git package source backend abstraction
#![allow(async_fn_in_trait)]

use crate::Project;
use crate::Url;
use crate::ser_display_deser_fromstr;
use crate::source::git_index::refresh_git_repo;
use crate::util::ToEscaped as _;
use relative_path::RelativePathBuf;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::task::spawn_blocking;
use tracing::instrument;

/// The tree ID type
pub type TreeId = Arc<str>;

/// Entry in a git tree
pub struct GitTreeEntry {
	/// The path of the entry
	pub path: RelativePathBuf,
	/// Whether the entry is a directory
	pub is_dir: bool,
	/// The object ID of the entry
	pub object_id: String,
}

/// A source of Git packages (low-level backend)
pub trait GitPackageSourceBackend: Debug + Display + Send + Sync {
	/// The error type for refreshing this backend
	type RefreshError: std::error::Error + Send + Sync + 'static;
	/// The error type for resolving a revision
	type ResolveRevError: std::error::Error + Send + Sync + 'static;
	/// The error type for reading a file
	type ReadFileError: std::error::Error + Send + Sync + 'static;
	/// The error type for listing a tree
	type ListTreeError: std::error::Error + Send + Sync + 'static;

	/// Refreshes the backend's index
	fn refresh(
		&self,
		project: &Project,
	) -> impl Future<Output = Result<(), Self::RefreshError>> + Send;

	/// Resolves a revision to a tree ID
	fn resolve_rev(
		&self,
		project: &Project,
		rev: String,
		path: Option<RelativePathBuf>,
	) -> impl Future<Output = Result<TreeId, Self::ResolveRevError>> + Send;

	/// Reads a file from the backend
	fn read_file(
		&self,
		project: &Project,
		tree_id: TreeId,
		file_path: RelativePathBuf,
	) -> impl Future<Output = Result<Option<Vec<u8>>, Self::ReadFileError>> + Send;

	/// Lists the entries in a tree
	fn list_tree(
		&self,
		project: &Project,
		tree_id: TreeId,
	) -> impl Future<Output = Result<Vec<GitTreeEntry>, Self::ListTreeError>> + Send;
}

/// A Git-based package source backend
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GixPackageSourceBackend {
	repo_url: Url,
}
ser_display_deser_fromstr!(GixPackageSourceBackend);

impl Display for GixPackageSourceBackend {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.repo_url)
	}
}

impl FromStr for GixPackageSourceBackend {
	type Err = crate::errors::ParseUrlError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self::new)
	}
}

impl GixPackageSourceBackend {
	/// Creates a new Git package source backend
	#[must_use]
	pub fn new(repo_url: Url) -> Self {
		Self { repo_url }
	}

	fn repo_path(&self, project: &Project) -> PathBuf {
		project
			.data_dir()
			.join("git_repos")
			.join("git")
			.join(self.repo_url.to_string().escaped())
	}

	/// Gets the repository URL
	#[must_use]
	pub fn repo_url(&self) -> &Url {
		&self.repo_url
	}
}

impl GitPackageSourceBackend for GixPackageSourceBackend {
	type RefreshError = crate::source::git_index::errors::RefreshIndexError;
	type ResolveRevError = errors::ResolveRevError;
	type ReadFileError = errors::ReadFileError;
	type ListTreeError = errors::ListTreeError;

	#[instrument(skip_all, level = "debug")]
	async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
		refresh_git_repo(self.repo_path(project), self.repo_url.clone()).await
	}

	async fn resolve_rev(
		&self,
		project: &Project,
		rev: String,
		path: Option<RelativePathBuf>,
	) -> Result<TreeId, Self::ResolveRevError> {
		let repo_path = self.repo_path(project);
		let repo_url = self.repo_url.clone();

		spawn_blocking(move || {
			let repo = gix::open(&repo_path)
				.map_err(|e| errors::ResolveRevErrorKind::OpenRepo(repo_url.clone(), e))?;

			let rev_obj = repo.rev_parse_single(&*rev).map_err(|e| {
				errors::ResolveRevErrorKind::ParseRev(rev.clone(), repo_url.clone(), e)
			})?;

			let root_tree = rev_obj
				.object()
				.map_err(|e| errors::ResolveRevErrorKind::ParseRevToObject(repo_url.clone(), e))?
				.peel_to_tree()
				.map_err(|e| errors::ResolveRevErrorKind::ParseObjectToTree(repo_url.clone(), e))?;

			let tree = if let Some(path) = &path {
				root_tree
					.lookup_entry_by_path(path.as_str())
					.map_err(|e| {
						errors::ResolveRevErrorKind::ReadTreeEntry(
							repo_url.clone(),
							path.clone(),
							e,
						)
					})?
					.ok_or_else(|| {
						errors::ResolveRevErrorKind::NoEntryAtPath(repo_url.clone(), path.clone())
					})?
					.object()
					.map_err(|e| {
						errors::ResolveRevErrorKind::ParseEntryToObject(repo_url.clone(), e)
					})?
					.peel_to_tree()
					.map_err(|e| {
						errors::ResolveRevErrorKind::ParseObjectToTree(repo_url.clone(), e)
					})?
			} else {
				root_tree
			};

			Ok(tree.id.to_string().into())
		})
		.await
		.unwrap()
	}

	async fn read_file(
		&self,
		project: &Project,
		tree_id: TreeId,
		file_path: RelativePathBuf,
	) -> Result<Option<Vec<u8>>, Self::ReadFileError> {
		let repo_path = self.repo_path(project);
		let repo_url = self.repo_url.clone();

		spawn_blocking(move || {
			let repo = gix::open(&repo_path)
				.map_err(|e| errors::ReadFileErrorKind::OpenRepo(repo_url.clone(), e))?;

			let tree_id = gix::ObjectId::from_hex(tree_id.as_bytes())
				.map_err(|e| errors::ReadFileErrorKind::ParseTreeId(repo_url.clone(), e))?;

			let tree = repo
				.find_object(tree_id)
				.map_err(|e| errors::ReadFileErrorKind::FindObject(repo_url.clone(), e))?
				.peel_to_tree()
				.map_err(|e| errors::ReadFileErrorKind::PeelToTree(repo_url.clone(), e))?;

			let entry = tree.lookup_entry_by_path(file_path.as_str()).map_err(|e| {
				errors::ReadFileErrorKind::LookupEntry(repo_url.clone(), file_path.clone(), e)
			})?;

			match entry {
				None => Ok(None),
				Some(entry) => {
					let mut object = entry.object().map_err(|e| {
						errors::ReadFileErrorKind::ReadObject(
							repo_url.clone(),
							file_path.clone(),
							e,
						)
					})?;
					Ok(Some(std::mem::take(&mut object.data)))
				}
			}
		})
		.await
		.unwrap()
	}

	async fn list_tree(
		&self,
		project: &Project,
		tree_id: TreeId,
	) -> Result<Vec<GitTreeEntry>, Self::ListTreeError> {
		let repo_path = self.repo_path(project);
		let repo_url = self.repo_url.clone();

		spawn_blocking(move || {
			let repo = gix::open(&repo_path)
				.map_err(|e| errors::ListTreeErrorKind::OpenRepo(repo_url.clone(), e))?;

			let tree_id = gix::ObjectId::from_hex(tree_id.as_bytes())
				.map_err(|e| errors::ListTreeErrorKind::ParseTreeId(repo_url.clone(), e))?;

			let tree = repo
				.find_object(tree_id)
				.map_err(|e| errors::ListTreeErrorKind::FindObject(repo_url.clone(), e))?
				.peel_to_tree()
				.map_err(|e| errors::ListTreeErrorKind::PeelToTree(repo_url.clone(), e))?;

			let mut recorder = gix::traverse::tree::Recorder::default();
			tree.traverse()
				.breadthfirst(&mut recorder)
				.map_err(|e| errors::ListTreeErrorKind::Traverse(repo_url.clone(), e))?;

			recorder
				.records
				.into_iter()
				.filter(|record| {
					// we do not support submodules, so we filter them out so
					// find_object does not error
					record.mode.kind() != gix::object::tree::EntryKind::Commit
				})
				.map(|record| {
					Ok(GitTreeEntry {
						path: RelativePathBuf::from(record.filepath.to_string()),
						is_dir: record.mode.is_tree(),
						object_id: record.oid.to_string(),
					})
				})
				.collect()
		})
		.await
		.unwrap()
	}
}

/// All available Git package backends
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum GitPackageBackends {
	/// A Git-based package source backend
	Git(GixPackageSourceBackend),
}
ser_display_deser_fromstr!(GitPackageBackends);

impl Display for GitPackageBackends {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Git(repo) => write!(f, "{repo}"),
		}
	}
}

impl FromStr for GitPackageBackends {
	type Err = errors::ParseBackendError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let git_err = match s.parse::<GixPackageSourceBackend>() {
			Ok(repo) => return Ok(Self::Git(repo)),
			Err(e) => e,
		};

		Err(errors::ParseBackendErrorKind::NoMatch(s.to_string(), git_err).into())
	}
}

impl GitPackageBackends {
	/// Gets the repository URL for this backend
	#[must_use]
	pub fn repo_url(&self) -> &Url {
		match self {
			Self::Git(repo) => repo.repo_url(),
		}
	}
}

impl GitPackageSourceBackend for GitPackageBackends {
	type RefreshError = crate::source::git_index::errors::RefreshIndexError;
	type ResolveRevError = errors::ResolveRevError;
	type ReadFileError = errors::ReadFileError;
	type ListTreeError = errors::ListTreeError;

	async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
		match self {
			Self::Git(repo) => repo.refresh(project).await,
		}
	}

	async fn resolve_rev(
		&self,
		project: &Project,
		rev: String,
		path: Option<RelativePathBuf>,
	) -> Result<TreeId, Self::ResolveRevError> {
		match self {
			Self::Git(repo) => repo.resolve_rev(project, rev, path).await,
		}
	}

	async fn read_file(
		&self,
		project: &Project,
		tree_id: TreeId,
		file_path: RelativePathBuf,
	) -> Result<Option<Vec<u8>>, Self::ReadFileError> {
		match self {
			Self::Git(repo) => repo.read_file(project, tree_id, file_path).await,
		}
	}

	async fn list_tree(
		&self,
		project: &Project,
		tree_id: TreeId,
	) -> Result<Vec<GitTreeEntry>, Self::ListTreeError> {
		match self {
			Self::Git(repo) => repo.list_tree(project, tree_id).await,
		}
	}
}

/// Errors that can occur when interacting with Git package source backends
pub mod errors {
	use relative_path::RelativePathBuf;
	use thiserror::Error;

	use crate::Url;

	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ParseBackendError))]
	/// Errors that can occur when parsing a Git package source backend
	pub enum ParseBackendErrorKind {
		/// No backend type matched the input
		#[error("no backend type matched for `{0}`")]
		NoMatch(String, #[source] crate::errors::ParseUrlError),
	}

	/// The error type for refreshing a Git package source backend
	pub type RefreshError = crate::source::git_index::errors::RefreshIndexError;

	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ResolveRevError))]
	#[non_exhaustive]
	/// Errors that can occur when resolving a revision in a Git package source
	pub enum ResolveRevErrorKind {
		/// An error occurred opening the repository
		#[error("error opening backend {0}")]
		OpenRepo(Url, #[source] gix::open::Error),

		/// An error occurred parsing a revision
		#[error("error parsing rev {0} in backend {1}")]
		ParseRev(
			String,
			Url,
			#[source] gix::revision::spec::parse::single::Error,
		),

		/// An error occurred parsing a revision to an object
		#[error("error parsing rev to object in backend {0}")]
		ParseRevToObject(Url, #[source] gix::object::find::existing::Error),

		/// An error occurred parsing an object to a tree in the backend
		#[error("error parsing object to tree in backend {0}")]
		ParseObjectToTree(Url, #[source] gix::object::peel::to_kind::Error),

		/// An error occurred reading a tree entry in the backend
		#[error("error reading tree entry at path {1} in backend {0}")]
		ReadTreeEntry(
			Url,
			RelativePathBuf,
			#[source] gix::object::find::existing::Error,
		),

		/// No entry was found at the specified path in the backend
		#[error("no entry at path {1} in backend {0}")]
		NoEntryAtPath(Url, RelativePathBuf),

		/// An error occurred parsing an entry to an object in the backend
		#[error("error parsing entry to object in backend {0}")]
		ParseEntryToObject(Url, #[source] gix::object::find::existing::Error),
	}

	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ReadFileError))]
	#[non_exhaustive]
	/// Errors that can occur when reading a file from a Git package source
	pub enum ReadFileErrorKind {
		/// An error occurred opening the repository
		#[error("error opening backend {0}")]
		OpenRepo(Url, #[source] gix::open::Error),

		/// An error occurred parsing a tree ID in the backend
		#[error("error parsing tree id in backend {0}")]
		ParseTreeId(Url, #[source] gix::hash::decode::Error),

		/// An error occurred finding an object in the backend
		#[error("error finding object in backend {0}")]
		FindObject(Url, #[source] gix::object::find::existing::Error),

		/// An error occurred peeling an object to a tree in the backend
		#[error("error peeling to tree in backend {0}")]
		PeelToTree(Url, #[source] gix::object::peel::to_kind::Error),

		/// An error occurred looking up an entry in the backend
		#[error("error looking up entry at path {1} in backend {0}")]
		LookupEntry(
			Url,
			RelativePathBuf,
			#[source] gix::object::find::existing::Error,
		),

		/// An error occurred reading an object in the backend
		#[error("error reading object at path {1} in backend {0}")]
		ReadObject(
			Url,
			RelativePathBuf,
			#[source] gix::object::find::existing::Error,
		),
	}

	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ListTreeError))]
	#[non_exhaustive]
	/// Errors that can occur when listing a tree from a Git package source
	pub enum ListTreeErrorKind {
		/// An error occurred opening the backend
		#[error("error opening backend {0}")]
		OpenRepo(Url, #[source] gix::open::Error),

		/// An error occurred parsing a tree ID in the backend
		#[error("error parsing tree id in backend {0}")]
		ParseTreeId(Url, #[source] gix::hash::decode::Error),

		/// An error occurred finding an object in the backend
		#[error("error finding object in backend {0}")]
		FindObject(Url, #[source] gix::object::find::existing::Error),

		/// An error occurred peeling an object to a tree in the backend
		#[error("error peeling to tree in backend {0}")]
		PeelToTree(Url, #[source] gix::object::peel::to_kind::Error),

		/// An error occurred traversing a tree in the backend
		#[error("error traversing tree in backend {0}")]
		Traverse(Url, #[source] gix::traverse::tree::breadthfirst::Error),
	}
}
