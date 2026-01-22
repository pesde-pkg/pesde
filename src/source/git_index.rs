#![allow(async_fn_in_trait)]

use crate::{GixUrl, Project, source::traits::RefreshOptions};
use fs_err::tokio as fs;
use gix::remote::Direction;
use std::fmt::Debug;
use tokio::task::spawn_blocking;
use tracing::instrument;

/// A trait for sources that are based on Git repositories
pub trait GitBasedSource {
	/// The path to the index
	fn path(&self, project: &Project) -> std::path::PathBuf;

	/// The URL of the repository
	fn repo_url(&self) -> &GixUrl;

	/// Refreshes the repository
	async fn refresh(&self, options: &RefreshOptions) -> Result<(), errors::RefreshError> {
		let path = self.path(&options.project);
		let repo_url = self.repo_url().clone();

		if fs::metadata(&path).await.is_ok() {
			spawn_blocking(move || {
				let repo = match gix::open_opts(&path, gix::open::Options::isolated()) {
					Ok(repo) => repo,
					Err(e) => return Err(errors::RefreshErrorKind::Open(path, e).into()),
				};
				let remote = match repo.find_default_remote(Direction::Fetch) {
					Some(Ok(remote)) => remote,
					Some(Err(e)) => {
						return Err(errors::RefreshErrorKind::GetDefaultRemote(path, e).into());
					}
					None => {
						return Err(errors::RefreshErrorKind::NoDefaultRemote(path).into());
					}
				};

				let connection = remote
					.connect(Direction::Fetch)
					.map_err(|e| errors::RefreshErrorKind::Connect(repo_url.clone(), e))?;

				let fetch = match connection
					.prepare_fetch(gix::progress::Discard, Default::default())
				{
					Ok(fetch) => fetch,
					Err(e) => {
						return Err(
							errors::RefreshErrorKind::PrepareFetch(repo_url.clone(), e).into()
						);
					}
				};

				match fetch.receive(gix::progress::Discard, &false.into()) {
					Ok(_) => Ok::<_, errors::RefreshError>(()),
					Err(e) => Err(errors::RefreshErrorKind::Read(repo_url.clone(), e).into()),
				}
			})
			.await
			.unwrap()?;

			return Ok(());
		}

		fs::create_dir_all(&path).await?;

		spawn_blocking(move || {
			gix::clone::PrepareFetch::new(
				repo_url.as_url().clone(),
				path,
				gix::create::Kind::Bare,
				gix::create::Options::default(),
				gix::open::Options::isolated(),
			)
			.map_err(|e| {
				errors::RefreshError::from(errors::RefreshErrorKind::Clone(repo_url.clone(), e))
			})?
			.fetch_only(gix::progress::Discard, &false.into())
			.map_err(|e| {
				errors::RefreshError::from(errors::RefreshErrorKind::Fetch(repo_url.clone(), e))
			})
		})
		.await
		.unwrap()
		.map(|_| ())
	}
}

/// Reads a file from a tree
#[instrument(skip(tree), ret, level = "trace")]
pub fn read_file<I: IntoIterator<Item = P> + Debug, P: ToString + PartialEq<gix::bstr::BStr>>(
	tree: &gix::Tree,
	file_path: I,
) -> Result<Option<String>, errors::ReadFile> {
	let mut file_path_str = String::new();

	let entry = match tree.lookup_entry(file_path.into_iter().inspect(|path| {
		file_path_str.push_str(path.to_string().as_str());
		file_path_str.push('/');
	})) {
		Ok(Some(entry)) => entry,
		Ok(None) => return Ok(None),
		Err(e) => return Err(errors::ReadFileKind::Lookup(file_path_str, e).into()),
	};

	let object = match entry.object() {
		Ok(object) => object,
		Err(e) => return Err(errors::ReadFileKind::Lookup(file_path_str, e).into()),
	};

	let blob = object.into_blob();
	let string = String::from_utf8(blob.data.clone())
		.map_err(|e| errors::ReadFileKind::Utf8(file_path_str, e))?;

	Ok(Some(string))
}

/// Gets the root tree of a repository
#[instrument(skip(repo), level = "trace")]
pub fn root_tree(repo: &gix::Repository) -> Result<gix::Tree<'_>, errors::TreeError> {
	// this is a bare repo, so this is the actual path
	let path = repo.path().to_path_buf();

	let remote = match repo.find_default_remote(Direction::Fetch) {
		Some(Ok(remote)) => remote,
		Some(Err(e)) => return Err(errors::TreeErrorKind::GetDefaultRemote(path, e).into()),
		None => {
			return Err(errors::TreeErrorKind::NoDefaultRemote(path).into());
		}
	};

	let Some(refspec) = remote.refspecs(Direction::Fetch).first() else {
		return Err(errors::TreeErrorKind::NoRefSpecs(path).into());
	};

	let spec_ref = refspec.to_ref();
	let local_ref = match spec_ref.local() {
		Some(local) => local
			.to_string()
			.replace('*', repo.branch_names().first().unwrap_or(&"main")),
		None => return Err(errors::TreeErrorKind::NoLocalRefSpec(path).into()),
	};

	let reference = match repo.find_reference(&local_ref) {
		Ok(reference) => reference,
		Err(e) => return Err(errors::TreeErrorKind::NoReference(local_ref.clone(), e).into()),
	};

	let reference_name = reference.name().as_bstr().to_string();
	let id = match reference.into_fully_peeled_id() {
		Ok(id) => id,
		Err(e) => return Err(errors::TreeErrorKind::CannotPeel(reference_name, e).into()),
	};

	let id_str = id.to_string();
	let object = match id.object() {
		Ok(object) => object,
		Err(e) => return Err(errors::TreeErrorKind::CannotConvertToObject(id_str, e).into()),
	};

	match object.peel_to_tree() {
		Ok(tree) => Ok(tree),
		Err(e) => Err(errors::TreeErrorKind::CannotPeelToTree(id_str, e).into()),
	}
}

/// Errors that can occur when interacting with a git-based package source
pub mod errors {
	use std::path::PathBuf;

	use thiserror::Error;

	use crate::GixUrl;

	/// Errors that can occur when refreshing a git-based package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = RefreshError))]
	#[non_exhaustive]
	pub enum RefreshErrorKind {
		/// Error interacting with the filesystem
		#[error("error interacting with the filesystem")]
		Io(#[from] std::io::Error),

		/// Error opening the repository
		#[error("error opening repository at {0}")]
		Open(PathBuf, #[source] gix::open::Error),

		/// No default remote found in repository
		#[error("no default remote found in repository at {0}")]
		NoDefaultRemote(PathBuf),

		/// Error getting default remote from repository
		#[error("error getting default remote from repository at {0}")]
		GetDefaultRemote(PathBuf, #[source] gix::remote::find::existing::Error),

		/// Error connecting to remote repository
		#[error("error connecting to remote repository at {0}")]
		Connect(GixUrl, #[source] gix::remote::connect::Error),

		/// Error preparing fetch from remote repository
		#[error("error preparing fetch from remote repository at {0}")]
		PrepareFetch(GixUrl, #[source] gix::remote::fetch::prepare::Error),

		/// Error reading from remote repository
		#[error("error reading from remote repository at {0}")]
		Read(GixUrl, #[source] gix::remote::fetch::Error),

		/// Error cloning repository
		#[error("error cloning repository from {0}")]
		Clone(GixUrl, #[source] gix::clone::Error),

		/// Error fetching repository
		#[error("error fetching repository from {0}")]
		Fetch(GixUrl, #[source] gix::clone::fetch::Error),
	}

	/// Errors that can occur when reading a git-based package source's tree
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = TreeError))]
	#[non_exhaustive]
	pub enum TreeErrorKind {
		/// Error interacting with the filesystem
		#[error("error interacting with the filesystem")]
		Io(#[from] std::io::Error),

		/// No default remote found in repository
		#[error("no default remote found in repository at {0}")]
		NoDefaultRemote(PathBuf),

		/// Error getting default remote from repository
		#[error("error getting default remote from repository at {0}")]
		GetDefaultRemote(PathBuf, #[source] gix::remote::find::existing::Error),

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
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ReadFile))]
	#[non_exhaustive]
	pub enum ReadFileKind {
		/// Error looking up entry in tree
		#[error("error looking up entry {0} in tree")]
		Lookup(String, #[source] gix::object::find::existing::Error),

		/// Error reading file as utf8
		#[error("error parsing file for {0} as utf8")]
		Utf8(String, #[source] std::string::FromUtf8Error),
	}
}
