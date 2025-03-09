#![warn(missing_docs)]
//! A package manager for the Luau programming language, supporting multiple runtimes including Roblox and Lune.
//! pesde has its own registry, however it can also use Wally, and Git repositories as package sources.
//! It has been designed with multiple targets in mind, namely Roblox, Lune, and Luau.

use crate::{
	lockfile::Lockfile,
	manifest::{target::TargetKind, Manifest},
	source::{
		traits::{PackageSource as _, RefreshOptions},
		PackageSources,
	},
};
use async_stream::try_stream;
use fs_err::tokio as fs;
use futures::Stream;
use gix::sec::identity::Account;
use semver::{Version, VersionReq};
use std::{
	collections::{HashMap, HashSet},
	fmt::Debug,
	hash::{Hash as _, Hasher as _},
	path::{Path, PathBuf},
	sync::Arc,
};
use tokio::io::AsyncReadExt as _;
use tracing::instrument;
use wax::Pattern as _;

/// Downloading packages
pub mod download;
/// Utility for downloading and linking in the correct order
pub mod download_and_link;
/// Handling of engines
pub mod engine;
/// Graphs
pub mod graph;
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
pub mod reporters;
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
/// The folder in which scripts are linked
pub const SCRIPTS_LINK_FOLDER: &str = ".pesde";

pub(crate) fn default_index_name() -> String {
	DEFAULT_INDEX_NAME.into()
}

#[derive(Debug, Default)]
struct AuthConfigShared {
	tokens: HashMap<gix::Url, String>,
	git_credentials: Option<Account>,
}

/// Struct containing the authentication configuration
#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
	shared: Arc<AuthConfigShared>,
}

impl AuthConfig {
	/// Create a new `AuthConfig`
	#[must_use]
	pub fn new() -> Self {
		AuthConfig::default()
	}

	/// Set the tokens
	/// Panics if the `AuthConfig` is shared
	#[must_use]
	pub fn with_tokens<I: IntoIterator<Item = (gix::Url, S)>, S: AsRef<str>>(
		mut self,
		tokens: I,
	) -> Self {
		Arc::get_mut(&mut self.shared).unwrap().tokens = tokens
			.into_iter()
			.map(|(url, s)| (url, s.as_ref().to_string()))
			.collect();
		self
	}

	/// Set the git credentials
	/// Panics if the `AuthConfig` is shared
	#[must_use]
	pub fn with_git_credentials(mut self, git_credentials: Option<Account>) -> Self {
		Arc::get_mut(&mut self.shared).unwrap().git_credentials = git_credentials;
		self
	}

	/// Get the tokens
	#[must_use]
	pub fn tokens(&self) -> &HashMap<gix::Url, String> {
		&self.shared.tokens
	}

	/// Get the git credentials
	#[must_use]
	pub fn git_credentials(&self) -> Option<&Account> {
		self.shared.git_credentials.as_ref()
	}
}

#[derive(Debug)]
struct ProjectShared {
	package_dir: PathBuf,
	workspace_dir: Option<PathBuf>,
	data_dir: PathBuf,
	cas_dir: PathBuf,
	auth_config: AuthConfig,
}

/// The main struct of the pesde library, representing a project
/// Unlike `ProjectShared`, this struct is `Send` and `Sync` and is cheap to clone because it is `Arc`-backed
#[derive(Debug, Clone)]
pub struct Project {
	shared: Arc<ProjectShared>,
}

impl Project {
	/// Create a new `Project`
	#[must_use]
	pub fn new(
		package_dir: impl AsRef<Path>,
		workspace_dir: Option<impl AsRef<Path>>,
		data_dir: impl AsRef<Path>,
		cas_dir: impl AsRef<Path>,
		auth_config: AuthConfig,
	) -> Self {
		Project {
			shared: Arc::new(ProjectShared {
				package_dir: package_dir.as_ref().to_path_buf(),
				workspace_dir: workspace_dir.map(|d| d.as_ref().to_path_buf()),
				data_dir: data_dir.as_ref().to_path_buf(),
				cas_dir: cas_dir.as_ref().to_path_buf(),
				auth_config,
			}),
		}
	}

	/// The directory of the package
	#[must_use]
	pub fn package_dir(&self) -> &Path {
		&self.shared.package_dir
	}

	/// The directory of the workspace this package belongs to, if any
	#[must_use]
	pub fn workspace_dir(&self) -> Option<&Path> {
		self.shared.workspace_dir.as_deref()
	}

	/// The directory to store general-purpose data
	#[must_use]
	pub fn data_dir(&self) -> &Path {
		&self.shared.data_dir
	}

	/// The CAS (content-addressable storage) directory
	#[must_use]
	pub fn cas_dir(&self) -> &Path {
		&self.shared.cas_dir
	}

	/// The authentication configuration
	#[must_use]
	pub fn auth_config(&self) -> &AuthConfig {
		&self.shared.auth_config
	}

	/// Read the manifest file
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub async fn read_manifest(&self) -> Result<String, errors::ManifestReadError> {
		let string = fs::read_to_string(self.package_dir().join(MANIFEST_FILE_NAME)).await?;
		Ok(string)
	}

	// TODO: cache the manifest
	/// Deserialize the manifest file
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub async fn deser_manifest(&self) -> Result<Manifest, errors::ManifestReadError> {
		deser_manifest(self.package_dir()).await
	}

	/// Deserialize the manifest file of the workspace root
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub async fn deser_workspace_manifest(
		&self,
	) -> Result<Option<Manifest>, errors::ManifestReadError> {
		let Some(workspace_dir) = self.workspace_dir() else {
			return Ok(None);
		};

		deser_manifest(workspace_dir).await.map(Some)
	}

	/// Write the manifest file
	#[instrument(skip(self, manifest), level = "debug")]
	pub async fn write_manifest<S: AsRef<[u8]>>(&self, manifest: S) -> Result<(), std::io::Error> {
		fs::write(
			self.package_dir().join(MANIFEST_FILE_NAME),
			manifest.as_ref(),
		)
		.await
	}

	/// Deserialize the lockfile
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub async fn deser_lockfile(&self) -> Result<Lockfile, errors::LockfileReadError> {
		let string = fs::read_to_string(self.package_dir().join(LOCKFILE_FILE_NAME)).await?;
		Ok(match toml::from_str(&string) {
			Ok(lockfile) => lockfile,
			Err(e) => {
				#[allow(deprecated)]
				let Ok(old_lockfile) = toml::from_str::<lockfile::old::LockfileOld>(&string) else {
					return Err(errors::LockfileReadError::Serde(e));
				};

				#[allow(deprecated)]
				old_lockfile.to_new()
			}
		})
	}

	/// Write the lockfile
	#[instrument(skip(self, lockfile), level = "debug")]
	pub async fn write_lockfile(
		&self,
		lockfile: &Lockfile,
	) -> Result<(), errors::LockfileWriteError> {
		let lockfile = toml::to_string(lockfile)?;
		let lockfile = format!(
			r"# This file is automatically @generated by pesde.
# It is not intended for manual editing.
{lockfile}"
		);

		fs::write(self.package_dir().join(LOCKFILE_FILE_NAME), lockfile).await?;
		Ok(())
	}

	/// Get the workspace members
	#[instrument(skip(self), level = "debug")]
	pub async fn workspace_members(
		&self,
		can_ref_self: bool,
	) -> Result<
		impl Stream<Item = Result<(PathBuf, Manifest), errors::WorkspaceMembersError>>,
		errors::WorkspaceMembersError,
	> {
		let dir = self.workspace_dir().unwrap_or(self.package_dir());
		let manifest = deser_manifest(dir).await?;

		let members = matching_globs(
			dir,
			manifest.workspace_members.iter().map(String::as_str),
			false,
			can_ref_self,
		)
		.await?;

		Ok(try_stream! {
			for path in members {
				let manifest = deser_manifest(&path).await?;
				yield (path, manifest);
			}
		})
	}
}

/// Gets all matching paths in a directory
#[instrument(ret, level = "trace")]
pub async fn matching_globs<'a, P: AsRef<Path> + Debug, I: IntoIterator<Item = &'a str> + Debug>(
	dir: P,
	globs: I,
	relative: bool,
	can_ref_self: bool,
) -> Result<HashSet<PathBuf>, errors::MatchingGlobsError> {
	let (negative_globs, mut positive_globs): (HashSet<&str>, _) =
		globs.into_iter().partition(|glob| glob.starts_with('!'));

	let include_self = positive_globs.remove(".") && can_ref_self;

	let negative_globs = wax::any(
		negative_globs
			.into_iter()
			.map(|glob| wax::Glob::new(&glob[1..]))
			.collect::<Result<Vec<_>, _>>()?,
	)?;
	let positive_globs = wax::any(
		positive_globs
			.into_iter()
			.map(wax::Glob::new)
			.collect::<Result<Vec<_>, _>>()?,
	)?;

	let mut read_dirs = vec![fs::read_dir(dir.as_ref().to_path_buf()).await?];
	let mut paths = HashSet::new();

	if include_self {
		paths.insert(if relative {
			PathBuf::new()
		} else {
			dir.as_ref().to_path_buf()
		});
	}

	while let Some(mut read_dir) = read_dirs.pop() {
		while let Some(entry) = read_dir.next_entry().await? {
			let path = entry.path();
			if entry.file_type().await?.is_dir() {
				read_dirs.push(fs::read_dir(&path).await?);
			}

			let relative_path = path.strip_prefix(dir.as_ref()).unwrap();

			if positive_globs.is_match(relative_path) && !negative_globs.is_match(relative_path) {
				paths.insert(if relative {
					relative_path.to_path_buf()
				} else {
					path.clone()
				});
			}
		}
	}

	Ok(paths)
}

/// A struct containing sources already having been refreshed
#[derive(Debug, Clone, Default)]
pub struct RefreshedSources(Arc<tokio::sync::Mutex<HashSet<u64>>>);

impl RefreshedSources {
	/// Create a new empty `RefreshedSources`
	#[must_use]
	pub fn new() -> Self {
		RefreshedSources::default()
	}

	/// Refreshes the source asynchronously if it has not already been refreshed.
	/// Will prevent more refreshes of the same source.
	pub async fn refresh(
		&self,
		source: &PackageSources,
		options: &RefreshOptions,
	) -> Result<(), source::errors::RefreshError> {
		let mut hasher = std::hash::DefaultHasher::new();
		source.hash(&mut hasher);
		let hash = hasher.finish();

		let mut refreshed_sources = self.0.lock().await;

		if refreshed_sources.insert(hash) {
			source.refresh(options).await
		} else {
			Ok(())
		}
	}
}

async fn deser_manifest(path: &Path) -> Result<Manifest, errors::ManifestReadError> {
	let string = fs::read_to_string(path.join(MANIFEST_FILE_NAME)).await?;
	toml::from_str(&string).map_err(|e| errors::ManifestReadError::Serde(path.to_path_buf(), e))
}

/// Find the project & workspace directory roots
pub async fn find_roots(
	cwd: PathBuf,
) -> Result<(PathBuf, Option<PathBuf>), errors::FindRootsError> {
	let mut current_path = Some(cwd.clone());
	let mut project_root = None::<PathBuf>;
	let mut workspace_dir = None::<PathBuf>;

	async fn get_workspace_members(
		manifest_file: &mut fs::File,
		path: &Path,
	) -> Result<HashSet<PathBuf>, errors::FindRootsError> {
		let mut manifest = String::new();
		manifest_file
			.read_to_string(&mut manifest)
			.await
			.map_err(errors::ManifestReadError::Io)?;
		let manifest: Manifest = toml::from_str(&manifest)
			.map_err(|e| errors::ManifestReadError::Serde(path.to_path_buf(), e))?;

		if manifest.workspace_members.is_empty() {
			return Ok(HashSet::new());
		}

		matching_globs(
			path,
			manifest.workspace_members.iter().map(String::as_str),
			false,
			false,
		)
		.await
		.map_err(errors::FindRootsError::Globbing)
	}

	while let Some(path) = current_path {
		current_path = path.parent().map(Path::to_path_buf);

		if workspace_dir.is_some() {
			if let Some(project_root) = project_root {
				return Ok((project_root, workspace_dir));
			}
		}

		let mut manifest = match fs::File::open(path.join(MANIFEST_FILE_NAME)).await {
			Ok(manifest) => manifest,
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
			Err(e) => return Err(errors::ManifestReadError::Io(e).into()),
		};

		match (project_root.as_ref(), workspace_dir.as_ref()) {
			(Some(project_root), None) => {
				if get_workspace_members(&mut manifest, &path)
					.await?
					.contains(project_root)
				{
					workspace_dir = Some(path);
				}
			}

			(None, None) => {
				if get_workspace_members(&mut manifest, &path)
					.await?
					.contains(&cwd)
				{
					// initializing a new member of a workspace
					return Ok((cwd, Some(path)));
				}

				project_root = Some(path);
			}

			(_, _) => unreachable!(),
		}
	}

	// we mustn't expect the project root to be found, as that would
	// disable the ability to run pesde in a non-project directory (for example to init it)
	Ok((project_root.unwrap_or(cwd), workspace_dir))
}

/// Returns whether a version matches a version requirement
/// Differs from `VersionReq::matches` in that EVERY version matches `*`
#[must_use]
pub fn version_matches(req: &VersionReq, version: &Version) -> bool {
	*req == VersionReq::STAR || req.matches(version)
}

pub(crate) fn all_packages_dirs() -> HashSet<String> {
	let mut dirs = HashSet::new();
	for target_kind_a in TargetKind::VARIANTS {
		for target_kind_b in TargetKind::VARIANTS {
			dirs.insert(target_kind_a.packages_folder(*target_kind_b));
		}
	}
	dirs
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
		#[error("error deserializing manifest file at {0}")]
		Serde(PathBuf, #[source] toml::de::Error),
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
		/// An error occurred parsing the manifest file
		#[error("error parsing manifest file")]
		ManifestParse(#[from] ManifestReadError),

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

		/// An error occurred while building a glob
		#[error("error building glob")]
		BuildGlob(#[from] wax::BuildError),
	}

	/// Errors that can occur when finding project roots
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum FindRootsError {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] ManifestReadError),

		/// Globbing failed
		#[error("error globbing")]
		Globbing(#[from] MatchingGlobsError),
	}
}
