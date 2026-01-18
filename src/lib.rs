#![warn(missing_docs)]
//! A package manager for the Luau programming language, supporting multiple runtimes including Roblox and Lune.
//! pesde has its own registry, however it can also use Wally, and Git repositories as package sources.
//! It has been designed with multiple targets in mind, namely Roblox, Lune, and Luau.

use crate::{
	lockfile::Lockfile,
	manifest::Manifest,
	source::{
		PackageSources,
		traits::{PackageSource as _, RefreshOptions},
	},
	util::hash,
};
use async_stream::try_stream;
use fs_err::tokio as fs;
use futures::Stream;
use gix::bstr::ByteSlice as _;
use relative_path::RelativePathBuf;
use semver::{Version, VersionReq};
use std::{
	collections::{HashMap, HashSet},
	fmt::{Debug, Display, Formatter},
	hash::{Hash as _, Hasher as _},
	path::{Path, PathBuf},
	str::FromStr,
	sync::Arc,
};
use tokio::{
	io::AsyncReadExt as _,
	sync::{OwnedRwLockReadGuard, RwLock},
};
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

pub(crate) fn default_index_name() -> String {
	DEFAULT_INDEX_NAME.into()
}

#[derive(Debug, Default)]
struct AuthConfigShared {
	tokens: HashMap<GixUrl, String>,
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
	pub fn with_tokens<I: IntoIterator<Item = (GixUrl, S)>, S: AsRef<str>>(
		mut self,
		tokens: I,
	) -> Self {
		Arc::get_mut(&mut self.shared).unwrap().tokens = tokens
			.into_iter()
			.map(|(url, s)| (url, s.as_ref().to_string()))
			.collect();
		self
	}

	/// Get the tokens
	#[must_use]
	pub fn tokens(&self) -> &HashMap<GixUrl, String> {
		&self.shared.tokens
	}
}

#[derive(Debug)]
struct ProjectShared {
	package_dir: PathBuf,
	workspace_dir: Option<PathBuf>,
	private_dir: PathBuf,
	data_dir: PathBuf,
	cas_dir: PathBuf,
	auth_config: AuthConfig,
}

/// The main struct of the pesde library, representing a project
/// Unlike `ProjectShared`, this struct is `Send` and `Sync` and is cheap to clone because it is `Arc`-backed
#[derive(Debug, Clone)]
pub struct Project {
	shared: Arc<ProjectShared>,
	manifest: Arc<RwLock<Option<Manifest>>>,
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
		let package_dir = package_dir.as_ref().to_path_buf();
		let workspace_dir = workspace_dir.map(|d| d.as_ref().to_path_buf());
		let cas_dir = cas_dir.as_ref().to_path_buf();

		Project {
			shared: ProjectShared {
				private_dir: cas_dir.join("projects").join(hash(
					workspace_dir
						.as_deref()
						.unwrap_or(&package_dir)
						.as_os_str()
						.as_encoded_bytes(),
				)),
				package_dir,
				workspace_dir,
				cas_dir,
				data_dir: data_dir.as_ref().to_path_buf(),
				auth_config,
			}
			.into(),
			manifest: Arc::new(RwLock::new(None)),
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

	/// The directory in which private, that is, non-shared data (dependencies, bins, etc.) is stored
	#[must_use]
	pub fn private_dir(&self) -> &Path {
		&self.shared.private_dir
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

	/// The directory in which the workspace resides, or the package directory if not in a workspace (or the workspace root)
	#[must_use]
	pub fn root_dir(&self) -> &Path {
		self.workspace_dir().unwrap_or(self.package_dir())
	}

	/// The path from the root directory to the package directory
	#[must_use]
	pub fn path_from_root(&self) -> RelativePathBuf {
		if let Some(workspace_dir) = &self.shared.workspace_dir {
			RelativePathBuf::from_path(self.shared.package_dir.strip_prefix(workspace_dir).unwrap())
				.unwrap()
		} else {
			RelativePathBuf::new()
		}
	}

	/// The project at [the root directory](Self::root_dir)
	#[must_use]
	pub fn into_root_project(self) -> Self {
		if let Some(workspace_dir) = &self.shared.workspace_dir {
			Project::new(
				workspace_dir,
				None::<PathBuf>,
				self.data_dir(),
				self.cas_dir(),
				self.auth_config().clone(),
			)
		} else {
			self
		}
	}

	/// Read the manifest file
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub async fn read_manifest(&self) -> Result<String, errors::ManifestReadError> {
		let string = fs::read_to_string(self.package_dir().join(MANIFEST_FILE_NAME)).await?;
		Ok(string)
	}

	/// Deserialize the manifest file
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub async fn deser_manifest(
		&self,
	) -> Result<OwnedRwLockReadGuard<Option<Manifest>, Manifest>, errors::ManifestReadError> {
		{
			let manifest_guard = self.manifest.clone().read_owned().await;
			if manifest_guard.is_some() {
				return Ok(OwnedRwLockReadGuard::map(manifest_guard, |m| {
					m.as_ref().unwrap()
				}));
			}
		}
		let mut manifest_guard = self.manifest.clone().write_owned().await;
		let manifest = toml::from_str::<Manifest>(
			&fs::read_to_string(self.package_dir().join(MANIFEST_FILE_NAME)).await?,
		)
		.map_err(|e| errors::ManifestReadError::Serde(self.package_dir().into(), e))?;
		*manifest_guard = Some(manifest);
		Ok(OwnedRwLockReadGuard::map(manifest_guard.downgrade(), |m| {
			m.as_ref().unwrap()
		}))
	}

	/// Write the manifest file
	#[instrument(skip(self, manifest), level = "debug")]
	pub async fn write_manifest(&self, manifest: impl AsRef<[u8]>) -> Result<(), std::io::Error> {
		*self.manifest.write().await = None;
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
		lockfile::parse_lockfile(&string).map_err(Into::into)
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
format = {}
{lockfile}",
			lockfile::CURRENT_FORMAT
		);

		fs::write(self.package_dir().join(LOCKFILE_FILE_NAME), lockfile).await?;
		Ok(())
	}

	/// Get the workspace members
	#[instrument(skip(self), level = "debug")]
	pub async fn workspace_members(
		&self,
	) -> Result<
		impl Stream<Item = Result<(RelativePathBuf, Manifest), errors::WorkspaceMembersError>>,
		errors::WorkspaceMembersError,
	> {
		let dir = self.root_dir();
		let manifest: Manifest = toml::from_str(
			&fs::read_to_string(dir.join(MANIFEST_FILE_NAME))
				.await
				.map_err(errors::ManifestReadError::Io)?,
		)
		.map_err(|e| errors::ManifestReadError::Serde(dir.into(), e))?;

		let members = matching_globs(
			dir,
			manifest.workspace_members.iter().map(String::as_str),
			false,
			false,
		)
		.await?;

		Ok(try_stream! {
			yield (RelativePathBuf::new(), manifest);

			for path in members {
				let manifest = toml::from_str::<Manifest>(
					&fs::read_to_string(path.join(MANIFEST_FILE_NAME))
						.await
						.map_err(errors::ManifestReadError::Io)?,
				)
				.map_err(|e| errors::ManifestReadError::Serde(path.clone().into(), e))?;
				yield (RelativePathBuf::from_path(path.strip_prefix(dir).unwrap()).unwrap(), manifest);
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
			.map_err(|e| errors::ManifestReadError::Serde(path.into(), e))?;

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

		if workspace_dir.is_some()
			&& let Some(project_root) = project_root
		{
			return Ok((project_root, workspace_dir));
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

/// A git repo URL
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GixUrl(Arc<gix::Url>);
ser_display_deser_fromstr!(GixUrl);

impl GixUrl {
	/// Creates a new [GixUrl] from a [gix::Url]
	/// pesde assumes the following information about Git URLs:
	/// - they are case insensitive
	/// - .git at the end is optional (it is removed if present)
	///
	/// Additionally, URLs are expected to use the HTTPS scheme. Users may override this in their Git configuration,
	/// but pesde will always use HTTPS URLs internally (specifically to make overriding easier).
	#[must_use]
	pub fn new(mut url: gix::Url) -> Self {
		url.path.make_ascii_lowercase();
		if url.path.ends_with(b".git") {
			let len = url.path.len();
			url.path.truncate(len - b".git".len());
		}
		url.scheme = gix::url::Scheme::Https;
		Self(url.into())
	}

	/// Returns the underlying [gix::Url]
	#[must_use]
	pub fn as_url(&self) -> &gix::Url {
		&self.0
	}
}

impl FromStr for GixUrl {
	type Err = errors::GixUrlError;

	fn from_str(mut s: &str) -> Result<Self, Self::Err> {
		if s.contains("://") {
			let Some(stripped) = s.strip_prefix("https://") else {
				return Err(Self::Err::HasScheme);
			};

			tracing::warn!(
				"specifying schemes in git URLs is deprecated and will be removed in a future version of pesde. faulty URL: {s}",
			);
			s = stripped;
		}

		format!("https://{s}")
			.try_into()
			.map(GixUrl::new)
			.map_err(Into::into)
	}
}

impl Display for GixUrl {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		let url = self.as_url().to_bstring();
		write!(
			f,
			"{}",
			url.strip_prefix(b"https://").unwrap_or(&url).as_bstr()
		)
	}
}

/// Errors that can occur when using the pesde library
pub mod errors {
	use std::path::Path;
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
		Serde(Box<Path>, #[source] toml::de::Error),
	}

	/// Errors that can occur when reading the lockfile
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum LockfileReadError {
		/// An IO error occurred
		#[error("io error reading lockfile")]
		Io(#[from] std::io::Error),

		/// An error occurred while parsing the lockfile
		#[error("error parsing lockfile")]
		Parse(#[from] crate::lockfile::errors::ParseLockfileError),
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

	/// Errors that can occur when interacting with git URLs
	#[derive(Debug, Error)]
	pub enum GixUrlError {
		/// An error occurred while parsing the git URL
		#[error("error parsing git URL")]
		Parse(#[from] gix::url::parse::Error),

		/// The URL has a scheme which is not supported by pesde
		#[error("git URL has unsupported scheme")]
		HasScheme,
	}
}
