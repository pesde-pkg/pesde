#![warn(missing_docs)]
//! A package manager for the Luau programming language, supporting multiple runtimes including Roblox and Lune.
//! pesde has its own registry, however it can also use Wally, and Git repositories as package sources.
//! It has been designed with multiple targets in mind, namely Roblox, Lune, and Luau.

use crate::lockfile::Lockfile;
use crate::manifest::Manifest;
use crate::source::PackageSources;
use crate::source::traits::PackageSource as _;
use crate::source::traits::RefreshOptions;
use crate::util::hash;
use fs_err::tokio as fs;
use gix::bstr::ByteSlice as _;
use relative_path::RelativePath;
use relative_path::RelativePathBuf;
use semver::Version;
use semver::VersionReq;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::hash::Hash as _;
use std::hash::Hasher as _;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::AsyncReadExt as _;
use tokio::sync::Mutex;
use tokio::sync::OwnedRwLockReadGuard;
use tokio::sync::RwLock;
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
	pub fn with_tokens<I: IntoIterator<Item = (GixUrl, impl Into<String>)>>(
		mut self,
		tokens: I,
	) -> Self {
		Arc::get_mut(&mut self.shared).unwrap().tokens =
			tokens.into_iter().map(|(url, s)| (url, s.into())).collect();
		self
	}

	/// Get the tokens
	#[must_use]
	pub fn tokens(&self) -> &HashMap<GixUrl, String> {
		&self.shared.tokens
	}
}

/// A workspace member. Can be empty for the root project.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Importer(Arc<RelativePath>);

impl Importer {
	pub(crate) fn new(path: impl Into<Arc<RelativePath>>) -> Self {
		Self(path.into())
	}

	/// An importer pointing to the project root
	#[must_use]
	pub fn root() -> Self {
		Self(RelativePath::new("").into())
	}

	/// Whether this importer is the root importer
	#[must_use]
	pub fn is_root(&self) -> bool {
		self.as_path().as_str().is_empty()
	}

	/// The path of this importer
	#[must_use]
	fn as_path(&self) -> &RelativePath {
		&self.0
	}
}

impl Display for Importer {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		if self.is_root() {
			write!(f, "(root)")
		} else {
			write!(f, "{}", self.as_path())
		}
	}
}

#[derive(Debug)]
struct ProjectShared {
	dir: PathBuf,
	private_dir: PathBuf,
	data_dir: PathBuf,
	cas_dir: PathBuf,
	auth_config: AuthConfig,
	manifests: Mutex<HashMap<Importer, Arc<RwLock<Manifest>>>>,
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
		dir: impl Into<PathBuf>,
		data_dir: impl Into<PathBuf>,
		cas_dir: impl Into<PathBuf>,
		auth_config: AuthConfig,
	) -> Self {
		let dir = dir.into();
		let cas_dir = cas_dir.into();

		Project {
			shared: ProjectShared {
				private_dir: cas_dir
					.join("projects")
					.join(hash(dir.as_os_str().as_encoded_bytes())),
				dir,
				cas_dir,
				data_dir: data_dir.into(),
				auth_config,
				manifests: Default::default(),
			}
			.into(),
		}
	}

	/// The directory of this project
	#[must_use]
	pub fn dir(&self) -> &Path {
		&self.shared.dir
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

	/// Create a subproject for an importer
	#[must_use]
	pub fn subproject(self, importer: Importer) -> Subproject {
		Subproject {
			shared: Arc::new(SubprojectShared {
				project: self,
				importer,
			}),
		}
	}

	/// Deserialize the lockfile
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub async fn deser_lockfile(&self) -> Result<Lockfile, errors::LockfileReadError> {
		let string = fs::read_to_string(self.dir().join(LOCKFILE_FILE_NAME)).await?;
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

		fs::write(self.dir().join(LOCKFILE_FILE_NAME), lockfile).await?;
		Ok(())
	}
}

#[derive(Debug)]
struct SubprojectShared {
	project: Project,
	importer: Importer,
}

/// An importer within a [Project]
#[derive(Debug, Clone)]
pub struct Subproject {
	shared: Arc<SubprojectShared>,
}

impl Subproject {
	/// The parent project
	#[must_use]
	pub fn project(&self) -> &Project {
		&self.shared.project
	}

	/// The importer path
	#[must_use]
	pub fn importer(&self) -> &Importer {
		&self.shared.importer
	}

	/// The importer directory
	#[must_use]
	pub fn dir(&self) -> PathBuf {
		self.importer().as_path().to_path(self.project().dir())
	}

	/// The private directory for this importer
	#[must_use]
	pub fn private_dir(&self) -> PathBuf {
		self.importer()
			.as_path()
			.to_path(self.project().private_dir())
	}

	/// The dependencies directory
	#[must_use]
	pub fn dependencies_dir(&self) -> PathBuf {
		self.private_dir().join("dependencies")
	}

	/// The bin directory
	#[must_use]
	pub fn bin_dir(&self) -> PathBuf {
		self.private_dir().join("bin")
	}

	/// Read the manifest file
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub async fn read_manifest(&self) -> Result<String, errors::ManifestReadError> {
		let string = fs::read_to_string(self.dir().join(MANIFEST_FILE_NAME)).await?;
		Ok(string)
	}

	/// Deserialize the manifest file
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub async fn deser_manifest(
		&self,
	) -> Result<OwnedRwLockReadGuard<Manifest>, errors::ManifestReadError> {
		let mut manifests_guard = self.project().shared.manifests.lock().await;
		if !manifests_guard.contains_key(self.importer()) {
			let manifest = fs::read_to_string(self.dir().join(MANIFEST_FILE_NAME)).await?;
			let manifest = toml::from_str::<Manifest>(&manifest)
				.map_err(|e| errors::ManifestReadErrorKind::Serde(self.dir(), e))?;
			manifests_guard.insert(self.importer().clone(), Arc::new(RwLock::new(manifest)));
		}
		Ok(manifests_guard[self.importer()].clone().read_owned().await)
	}

	/// Write the manifest file
	#[instrument(skip(self, manifest), level = "debug")]
	pub async fn write_manifest(&self, manifest: impl AsRef<[u8]>) -> Result<(), std::io::Error> {
		self.project()
			.shared
			.manifests
			.lock()
			.await
			.remove(self.importer());
		fs::write(self.dir().join(MANIFEST_FILE_NAME), manifest.as_ref()).await
	}
}

/// Gets all matching paths in a directory
#[instrument(ret, level = "trace")]
pub async fn matching_globs<'a>(
	dir: impl AsRef<Path> + Debug,
	globs: impl IntoIterator<Item = &'a str> + Debug,
) -> Result<HashSet<PathBuf>, errors::MatchingGlobsError> {
	let (negative_globs, positive_globs): (Vec<&str>, _) =
		globs.into_iter().partition(|glob| glob.starts_with('!'));

	let negative_globs = wax::any(
		negative_globs
			.into_iter()
			.map(|glob| wax::Glob::new(&glob[1..]))
			.collect::<Result<Vec<_>, _>>()?,
	)?;
	let positive_globs = wax::any(
		positive_globs
			.into_iter()
			.filter(|glob| *glob != ".")
			.map(wax::Glob::new)
			.collect::<Result<Vec<_>, _>>()?,
	)?;

	let mut read_dirs = vec![fs::read_dir(dir.as_ref().to_path_buf()).await?];
	let mut paths = HashSet::new();

	while let Some(mut read_dir) = read_dirs.pop() {
		while let Some(entry) = read_dir.next_entry().await? {
			let path = entry.path();
			if entry.file_type().await?.is_dir() {
				read_dirs.push(fs::read_dir(&path).await?);
			}

			let relative_path = path.strip_prefix(dir.as_ref()).unwrap();

			if positive_globs.is_match(relative_path) && !negative_globs.is_match(relative_path) {
				paths.insert(path);
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
pub async fn find_roots(cwd: PathBuf) -> Result<(PathBuf, Importer), errors::FindRootsError> {
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
			.map_err(|e| errors::ManifestReadError::from(errors::ManifestReadErrorKind::Io(e)))?;
		let manifest: Manifest = toml::from_str(&manifest).map_err(|e| {
			errors::ManifestReadError::from(errors::ManifestReadErrorKind::Serde(path.into(), e))
		})?;

		if manifest.workspace.members.is_empty() {
			return Ok(HashSet::new());
		}

		matching_globs(path, manifest.workspace.members.iter().map(String::as_str))
			.await
			.map_err(|e| errors::FindRootsErrorKind::Globbing(e).into())
	}

	macro_rules! to_importer {
		($project_root:ident, $workspace_root:ident) => {
			Importer::new(
				RelativePathBuf::from_path($project_root.strip_prefix(&$workspace_root).unwrap())
					.unwrap(),
			)
		};
	}

	while let Some(path) = current_path {
		current_path = path.parent().map(Path::to_path_buf);

		if let Some(workspace_dir) = workspace_dir.as_deref()
			&& let Some(project_root) = project_root
		{
			let importer = to_importer!(project_root, workspace_dir);
			return Ok((project_root, importer));
		}

		let mut manifest = match fs::File::open(path.join(MANIFEST_FILE_NAME)).await {
			Ok(manifest) => manifest,
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
			Err(e) => {
				return Err(
					errors::ManifestReadError::from(errors::ManifestReadErrorKind::Io(e)).into(),
				);
			}
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
					let importer = to_importer!(cwd, path);
					return Ok((cwd, importer));
				}

				project_root = Some(path);
			}

			(_, _) => unreachable!(),
		}
	}

	// we mustn't expect the project root to be found, as that would
	// disable the ability to run pesde in a non-project directory (for example to init it)
	let project_root = project_root.unwrap_or(cwd);
	let workspace_root = workspace_dir.as_deref().unwrap_or(&project_root);
	let importer = to_importer!(project_root, workspace_root);
	Ok((project_root, importer))
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
				return Err(errors::GixUrlErrorKind::HasScheme.into());
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
	use std::path::PathBuf;
	use thiserror::Error;

	/// Errors that can occur when reading the manifest file
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ManifestReadError))]
	#[non_exhaustive]
	pub enum ManifestReadErrorKind {
		/// An IO error occurred
		#[error("io error reading manifest file")]
		Io(#[from] std::io::Error),

		/// An error occurred while deserializing the manifest file
		#[error("error deserializing manifest file at {0}")]
		Serde(PathBuf, #[source] toml::de::Error),
	}

	/// Errors that can occur when reading the lockfile
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = LockfileReadError))]
	#[non_exhaustive]
	pub enum LockfileReadErrorKind {
		/// An IO error occurred
		#[error("io error reading lockfile")]
		Io(#[from] std::io::Error),

		/// An error occurred while parsing the lockfile
		#[error("error parsing lockfile")]
		Parse(#[from] crate::lockfile::errors::ParseLockfileError),
	}

	/// Errors that can occur when writing the lockfile
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = LockfileWriteError))]
	#[non_exhaustive]
	pub enum LockfileWriteErrorKind {
		/// An IO error occurred
		#[error("io error writing lockfile")]
		Io(#[from] std::io::Error),

		/// An error occurred while serializing the lockfile
		#[error("error serializing lockfile")]
		Serde(#[from] toml::ser::Error),
	}

	/// Errors that can occur when finding matching globs
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = MatchingGlobsError))]
	#[non_exhaustive]
	pub enum MatchingGlobsErrorKind {
		/// An error occurred interacting with the filesystem
		#[error("error interacting with the filesystem")]
		Io(#[from] std::io::Error),

		/// An error occurred while building a glob
		#[error("error building glob")]
		BuildGlob(#[from] wax::BuildError),
	}

	/// Errors that can occur when finding project roots
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = FindRootsError))]
	#[non_exhaustive]
	pub enum FindRootsErrorKind {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] ManifestReadError),

		/// Globbing failed
		#[error("error globbing")]
		Globbing(#[from] MatchingGlobsError),
	}

	/// Errors that can occur when interacting with git URLs
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GixUrlError))]
	pub enum GixUrlErrorKind {
		/// An error occurred while parsing the git URL
		#[error("error parsing git URL")]
		Parse(#[from] gix::url::parse::Error),

		/// The URL has a scheme which is not supported by pesde
		#[error("git URL has unsupported scheme")]
		HasScheme,
	}
}
