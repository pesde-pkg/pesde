//! Git package source
use crate::GixUrl;
use crate::MANIFEST_FILE_NAME;
use crate::Project;
use crate::RefreshedSources;
use crate::Subproject;
use crate::errors::ManifestReadError;
use crate::errors::ManifestReadErrorKind;
use crate::manifest::Alias;
use crate::manifest::DependencyType;
use crate::manifest::Manifest;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
use crate::source::ADDITIONAL_FORBIDDEN_FILES;
use crate::source::DependencySpecifiers;
use crate::source::IGNORED_DIRS;
use crate::source::IGNORED_FILES;
use crate::source::PackageExports;
use crate::source::PackageRefs;
use crate::source::PackageSource;
use crate::source::PackageSources;
use crate::source::ResolveResult;
use crate::source::ResolvedPackage;
use crate::source::StructureKind;
use crate::source::fs::FsEntry;
use crate::source::fs::PackageFs;
use crate::source::fs::store_in_cas;
use crate::source::git::pkg_ref::GitPackageRef;
use crate::source::git_index::GitBasedSource;
use crate::source::git_index::read_file;
use crate::source::path::RelativeOrAbsolutePath;
use crate::source::pesde::PesdeVersionedManifest;
use crate::source::wally::compat_util::WALLY_MANIFEST_FILE_NAME;
use crate::source::wally::compat_util::get_exports;
use crate::source::wally::manifest::WallyManifest;
use crate::util::simplify_path;
use fs_err::tokio as fs;
use gix::ObjectId;
use gix::traverse::tree::Recorder;
use relative_path::RelativePathBuf;
use semver::BuildMetadata;
use semver::Version;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fmt::Display;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio::task::spawn_blocking;
use tracing::instrument;

pub mod pkg_ref;
pub mod specifier;

/// The Git package source
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct GitPackageSource {
	repo_url: GixUrl,
}
ser_display_deser_fromstr!(GitPackageSource);

impl Display for GitPackageSource {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.repo_url)
	}
}

impl FromStr for GitPackageSource {
	type Err = crate::errors::GixUrlError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self::new)
	}
}

impl GitBasedSource for GitPackageSource {
	const INDEX_SCOPE: &'static str = "git";

	fn repo_url(&self) -> &GixUrl {
		&self.repo_url
	}
}

impl GitPackageSource {
	/// Creates a new Git package source
	#[must_use]
	pub fn new(repo_url: GixUrl) -> Self {
		Self { repo_url }
	}
}

fn transform_pesde_dependencies(
	subproject: &Subproject,
	manifest: &Manifest,
	repo_url: &GixUrl,
) -> Result<BTreeMap<Alias, (DependencySpecifiers, DependencyType)>, errors::ResolveError> {
	let dependencies = manifest
		.all_dependencies()
		.map_err(|e| errors::ResolveErrorKind::CollectDependencies(repo_url.clone(), e))?;

	dependencies
		.into_iter()
		.map(|(alias, (mut spec, ty))| {
			match &mut spec {
				DependencySpecifiers::Pesde(specifier) => {
					specifier.index = manifest
						.indices
						.pesde
						.get(&specifier.index)
						.ok_or_else(|| {
							errors::ResolveErrorKind::PesdeIndexNotFound(
								specifier.index.clone(),
								repo_url.clone(),
							)
						})?
						.to_string();
				}
				DependencySpecifiers::Wally(specifier) => {
					specifier.index = manifest
						.indices
						.wally
						.get(&specifier.index)
						.ok_or_else(|| {
							errors::ResolveErrorKind::WallyIndexNotFound(
								specifier.index.clone(),
								repo_url.clone(),
							)
						})?
						.to_string();
				}
				DependencySpecifiers::Git(_) => {}
				DependencySpecifiers::Path(specifier) => {
					if let RelativeOrAbsolutePath::Relative(path) = &specifier.path
						&& simplify_path(&path.to_path(subproject.dir()))
							.starts_with(subproject.dir())
					{
					} else if std::env::var("PESDE_IMPURE_GIT_DEP_PATHS")
						.is_ok_and(|s| !s.is_empty())
					{
					} else {
						return Err(errors::ResolveErrorKind::Path(repo_url.clone()).into());
					}
				}
			}

			Ok((alias, (spec, ty)))
		})
		.collect()
}

impl PackageSource for GitPackageSource {
	type RefreshError = errors::RefreshError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetExportsError = errors::GetExportsError;

	#[instrument(skip_all, level = "debug")]
	async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
		GitBasedSource::refresh(self, project).await
	}

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		subproject: &Subproject,
		specifier: &DependencySpecifiers,
		_refreshed_sources: &RefreshedSources,
	) -> Result<ResolveResult, Self::ResolveError> {
		let DependencySpecifiers::Git(specifier) = specifier else {
			unreachable!("invalid specifier type for Git package source");
		};

		let path = self.path(subproject.project());
		let repo_url = self.repo_url.clone();
		let specifier = specifier.clone();
		let subproject = subproject.clone();

		let (structure_kind, version, dependencies, tree_id) = spawn_blocking::<
			_,
			Result<_, Self::ResolveError>,
		>(move || {
			let repo = gix::open(path)
				.map_err(|e| errors::ResolveErrorKind::OpenRepo(repo_url.clone(), e))?;
			let rev = repo.rev_parse_single(&*specifier.rev).map_err(|e| {
				errors::ResolveErrorKind::ParseRev(specifier.rev.clone(), repo_url.clone(), e)
			})?;

			let root_tree = rev
				.object()
				.map_err(|e| errors::ResolveErrorKind::ParseRevToObject(repo_url.clone(), e))?
				.peel_to_tree()
				.map_err(|e| errors::ResolveErrorKind::ParseObjectToTree(repo_url.clone(), e))?;

			let tree = if let Some(path) = &specifier.path {
				root_tree
					.lookup_entry_by_path(path.as_str())
					.map_err(|e| {
						errors::ResolveErrorKind::ReadTreeEntry(repo_url.clone(), path.clone(), e)
					})?
					.ok_or_else(|| {
						errors::ResolveErrorKind::NoEntryAtPath(repo_url.clone(), path.clone())
					})?
					.object()
					.map_err(|e| errors::ResolveErrorKind::ParseEntryToObject(repo_url.clone(), e))?
					.peel_to_tree()
					.map_err(|e| errors::ResolveErrorKind::ParseObjectToTree(repo_url.clone(), e))?
			} else {
				root_tree
			};

			let tree_version = Version {
				major: 0,
				minor: 0,
				patch: 0,
				build: BuildMetadata::EMPTY,
				pre: tree.id.to_string().parse().unwrap(),
			};

			if let Some(m) = read_file(&tree, [MANIFEST_FILE_NAME])
				.map_err(|e| errors::ResolveErrorKind::ReadManifest(repo_url.clone(), e))?
			{
				let manifest = toml::from_str::<PesdeVersionedManifest>(&m)
					.map_err(|e| errors::ResolveErrorKind::DeserManifest(repo_url.clone(), e))?;
				return Ok((
					match &manifest {
						PesdeVersionedManifest::V1(m) => StructureKind::PesdeV1(m.target.kind()),
						PesdeVersionedManifest::V2(_) => StructureKind::PesdeV2,
					},
					tree_version,
					transform_pesde_dependencies(&subproject, manifest.as_manifest(), &repo_url)?,
					tree.id.to_string(),
				));
			}

			let manifest = read_file(&tree, [WALLY_MANIFEST_FILE_NAME])
				.map_err(|e| errors::ResolveErrorKind::ReadManifest(repo_url.clone(), e))?;

			let Some(manifest) = manifest else {
				return Err(errors::ResolveErrorKind::NoManifest(repo_url.clone()).into());
			};

			let manifest = match toml::from_str::<WallyManifest>(&manifest) {
				Ok(manifest) => manifest,
				Err(e) => {
					return Err(errors::ResolveErrorKind::DeserManifest(repo_url.clone(), e).into());
				}
			};
			let (package, dependencies) = manifest
				.into_resolve_entry()
				.map_err(|e| errors::ResolveErrorKind::CollectDependencies(repo_url.clone(), e))?;

			Ok((
				StructureKind::Wally(package.name.name().into()),
				tree_version,
				dependencies,
				tree.id.to_string(),
			))
		})
		.await
		.unwrap()?;

		Ok(ResolveResult {
			source: PackageSources::Git(self.clone()),
			pkg_ref: PackageRefs::Git(GitPackageRef { tree_id }),
			structure_kind,
			versions: BTreeMap::from([(version, dependencies)]),
		})
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter>(
		&self,
		project: &Project,
		package: &ResolvedPackage,
		reporter: Arc<R>,
	) -> Result<PackageFs, Self::DownloadError> {
		let PackageRefs::Git(pkg_ref) = package.id.pkg_ref() else {
			unreachable!("invalid package ref type for Git package source");
		};

		let index_file = project
			.cas_dir()
			.join("index")
			.join("git")
			.join(&pkg_ref.tree_id);

		let repo_url = self.repo_url.clone();

		match fs::read_to_string(&index_file).await {
			Ok(s) => {
				tracing::debug!(
					"using cached index file for package {}#{}",
					self.repo_url,
					pkg_ref.tree_id
				);
				reporter.report_done();
				return toml::from_str::<PackageFs>(&s)
					.map_err(|e| errors::DownloadErrorKind::DeserializeFile(repo_url, e).into());
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(errors::DownloadErrorKind::Io(e).into()),
		}

		let path = self.path(project);
		let tree_id = match pkg_ref.tree_id.parse::<ObjectId>() {
			Ok(oid) => oid,
			Err(e) => return Err(errors::DownloadErrorKind::ParseTreeId(repo_url, e).into()),
		};

		let records = spawn_blocking(move || {
			let repo = gix::open(path)
				.map_err(|e| errors::DownloadErrorKind::OpenRepo(repo_url.clone(), e))?;

			let mut recorder = Recorder::default();

			let object = match repo.find_object(tree_id) {
				Ok(object) => object,
				Err(e) => {
					return Err(errors::DownloadErrorKind::OidToTree(tree_id, repo_url, e).into());
				}
			};

			let tree = match object.peel_to_tree() {
				Ok(tree) => tree,
				Err(e) => {
					return Err(errors::DownloadErrorKind::ParseObjectToTree(repo_url, e).into());
				}
			};

			if let Err(e) = tree.traverse().breadthfirst(&mut recorder) {
				return Err(errors::DownloadErrorKind::TraverseTree(repo_url, e).into());
			}

			recorder
				.records
				.into_iter()
				.filter(|entry| {
					// we do not support submodules, so we filter them out so
					// find_object does not error
					entry.mode.kind() != gix::object::tree::EntryKind::Commit
				})
				.map(|entry| {
					let mut object = repo.find_object(entry.oid).map_err(|e| {
						errors::DownloadErrorKind::ParseOidToObject(entry.oid, repo_url.clone(), e)
					})?;

					Ok::<_, errors::DownloadError>((
						RelativePathBuf::from(entry.filepath.to_string()),
						if matches!(object.kind, gix::object::Kind::Tree) {
							None
						} else {
							Some(std::mem::take(&mut object.data))
						},
					))
				})
				.collect::<Result<Vec<_>, _>>()
		})
		.await
		.unwrap()?;

		let is_wally = package.structure_kind.is_wally();
		let mut tasks = records
			.into_iter()
			.filter(|(path, contents)| {
				let name = path.file_name().unwrap_or("");
				if contents.is_none() {
					return !IGNORED_DIRS.contains(&name);
				}

				if IGNORED_FILES.contains(&name) {
					return false;
				}

				if !is_wally && ADDITIONAL_FORBIDDEN_FILES.contains(&name) {
					tracing::debug!(
						"removing {name} from {}#{} at {path} - using new structure",
						self.repo_url,
						pkg_ref.tree_id
					);
					return false;
				}

				true
			})
			.map(|(path, contents)| {
				let project = project.clone();

				async move {
					let Some(contents) = contents else {
						return Ok::<_, errors::DownloadError>((path, FsEntry::Directory));
					};

					let (_, hash) = store_in_cas(project.cas_dir(), contents.as_slice()).await?;

					Ok((path, FsEntry::File(hash)))
				}
			})
			.collect::<JoinSet<_>>();

		let mut entries = BTreeMap::new();

		while let Some(res) = tasks.join_next().await {
			let (path, entry) = res.unwrap()?;
			entries.insert(path, entry);
		}

		let fs = PackageFs::Cached(entries);

		if let Some(parent) = index_file.parent() {
			fs::create_dir_all(parent).await?;
		}

		fs::write(
			&index_file,
			toml::to_string(&fs)
				.map_err(|e| errors::DownloadErrorKind::SerializeIndex(self.repo_url.clone(), e))?,
		)
		.await
		.map_err(errors::DownloadErrorKind::Io)?;

		reporter.report_done();

		Ok(fs)
	}

	#[instrument(skip_all, level = "debug")]
	async fn get_exports(
		&self,
		project: &Project,
		package: &ResolvedPackage,
		path: &Path,
	) -> Result<PackageExports, Self::GetExportsError> {
		if package.structure_kind.is_wally() {
			return get_exports(project, path).await.map_err(Into::into);
		}

		let manifest = fs::read_to_string(path.join(MANIFEST_FILE_NAME))
			.await
			.map_err(|e| ManifestReadError::from(ManifestReadErrorKind::Io(e)))?;
		let manifest: PesdeVersionedManifest = toml::from_str(&manifest).map_err(|e| {
			ManifestReadError::from(ManifestReadErrorKind::Serde(path.to_path_buf(), e))
		})?;

		Ok(manifest.as_exports())
	}
}

/// Errors that can occur when interacting with the Git package source
pub mod errors {
	use crate::GixUrl;
	use gix::ObjectId;
	use relative_path::RelativePathBuf;
	use thiserror::Error;

	/// Errors that can occur when refreshing the Git package source
	pub type RefreshError = crate::source::git_index::errors::RefreshError;

	/// Errors that can occur when resolving a package from a Git package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ResolveError))]
	#[non_exhaustive]
	pub enum ResolveErrorKind {
		/// An error occurred opening the Git repository
		#[error("error opening Git repository for url {0}")]
		OpenRepo(GixUrl, #[source] gix::open::Error),

		/// An error occurred parsing rev
		#[error("error parsing rev {0} for repository {1}")]
		ParseRev(
			String,
			GixUrl,
			#[source] gix::revision::spec::parse::single::Error,
		),

		/// An error occurred creating reference iterator
		#[error("error creating reference iterator for repository {0}")]
		RefIter(GixUrl, #[source] gix::reference::iter::Error),

		/// An error occurred setting up reference iterator
		#[error("error setting up reference iterator for repository {0}")]
		RefSetup(GixUrl, #[source] gix::reference::iter::init::Error),

		/// An error occurred converting rev to object id
		#[error("error converting rev to object id for repository {0}")]
		RevToId(String, GixUrl, #[source] gix::reference::peel::Error),

		/// An error occurred iterating references
		#[error("error iterating references for repository {0}")]
		IterRefs(GixUrl, #[source] Box<dyn std::error::Error + Send + Sync>),

		/// No matching version was found
		#[error("no matching version found for requirement {0} in repository {1}")]
		NoMatchingVersion(String, GixUrl),

		/// An error occurred parsing rev to object
		#[error("error parsing rev to object for repository {0}")]
		ParseRevToObject(GixUrl, #[source] gix::object::find::existing::Error),

		/// An error occurred parsing object to tree
		#[error("error parsing object to tree for repository {0}")]
		ParseObjectToTree(GixUrl, #[source] gix::object::peel::to_kind::Error),

		/// An error occurred reading the manifest
		#[error("error reading manifest of repository {0}")]
		ReadManifest(GixUrl, #[source] crate::source::git_index::errors::ReadFile),

		/// An error occurred collecting all manifest dependencies
		#[error("error collecting all manifest dependencies for repository {0}")]
		CollectDependencies(
			GixUrl,
			#[source] crate::manifest::errors::AllDependenciesError,
		),

		/// An error occurred deserializing a manifest
		#[error("error deserializing manifest for repository {0}")]
		DeserManifest(GixUrl, #[source] toml::de::Error),

		/// No manifest was found
		#[error("no manifest found in repository {0}")]
		NoManifest(GixUrl),

		/// A pesde index was not found in the manifest
		#[error("pesde index {0} not found in manifest for repository {1}")]
		PesdeIndexNotFound(String, GixUrl),

		/// A Wally index was not found in the manifest
		#[error("wally index {0} not found in manifest for repository {1}")]
		WallyIndexNotFound(String, GixUrl),

		/// An error occurred reading a tree entry
		#[error("error reading tree entry for repository {0} at {1}")]
		ReadTreeEntry(
			GixUrl,
			RelativePathBuf,
			#[source] gix::object::find::existing::Error,
		),

		/// No entry was found at the specified path
		#[error("no entry found at path {1} in repository {0}")]
		NoEntryAtPath(GixUrl, RelativePathBuf),

		/// An error occurred parsing an entry to object
		#[error("error parsing an entry to object for repository {0}")]
		ParseEntryToObject(GixUrl, #[source] gix::object::find::existing::Error),

		/// An error occurred reading the lockfile
		#[error("error reading lockfile for repository {0}")]
		ReadLockfile(GixUrl, #[source] crate::source::git_index::errors::ReadFile),

		/// An error occurred while parsing the lockfile
		#[error("error parsing lockfile for repository {0}")]
		ParseLockfile(
			GixUrl,
			#[source] crate::lockfile::errors::ParseLockfileError,
		),

		/// The repository is missing a lockfile
		#[error("no lockfile found in repository {0}")]
		NoLockfile(GixUrl),

		/// The package depends on a path package that escapes the repository
		#[error(
			"the package {0} depends on a path package that escapes the repository. use PESDE_IMPURE_GIT_DEP_PATHS to override"
		)]
		Path(GixUrl),
	}

	/// Errors that can occur when downloading a package from a Git package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		/// An error occurred deserializing a file
		#[error("error deserializing file in repository {0}")]
		DeserializeFile(GixUrl, #[source] toml::de::Error),

		/// An error occurred interacting with the file system
		#[error("error interacting with the file system")]
		Io(#[from] std::io::Error),

		/// An error occurred while getting Wally exports
		#[error("error getting wally exports")]
		WallyGetExports(#[from] crate::source::wally::compat_util::errors::GetExportsError),

		/// An error occurred opening the Git repository
		#[error("error opening Git repository for url {0}")]
		OpenRepo(GixUrl, #[source] gix::open::Error),

		/// An error occurred while traversing the tree
		#[error("error traversing tree for repository {0}")]
		TraverseTree(GixUrl, #[source] gix::traverse::tree::breadthfirst::Error),

		/// Getting the tree by object id failed
		#[error("error getting tree from object id {0} for repository {1}")]
		OidToTree(
			ObjectId,
			GixUrl,
			#[source] gix::object::find::existing::Error,
		),

		/// An error occurred parsing an object id to object
		#[error("error parsing object id {0} to object for repository {1}")]
		ParseOidToObject(
			ObjectId,
			GixUrl,
			#[source] gix::object::find::existing::Error,
		),

		/// An error occurred parsing object to tree
		#[error("error parsing object to tree for repository {0}")]
		ParseObjectToTree(GixUrl, #[source] gix::object::peel::to_kind::Error),

		/// An error occurred while serializing the index file
		#[error("error serializing the index file for repository {0}")]
		SerializeIndex(GixUrl, #[source] toml::ser::Error),

		/// An error occurred while parsing tree_id to ObjectId
		#[error("error parsing tree_id to ObjectId for repository {0}")]
		ParseTreeId(GixUrl, #[source] gix::hash::decode::Error),
	}

	/// Errors that can occur when getting a target from a Git package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GetExportsError))]
	#[non_exhaustive]
	pub enum GetExportsErrorKind {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// An error occurred while getting Wally exports
		#[error("error getting Wally exports")]
		WallyGetExports(#[from] crate::source::wally::compat_util::errors::GetExportsError),
	}
}
