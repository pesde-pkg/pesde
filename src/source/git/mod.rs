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
use crate::source::SourceState;
use crate::source::StructureKind;
use crate::source::fs::FsEntry;
use crate::source::fs::PackageFs;
use crate::source::fs::store_in_cas;
use crate::source::git::backend::GitPackageBackends;
use crate::source::git::backend::GitPackageSourceBackend as _;
use crate::source::git::backend::GixPackageSourceBackend;
use crate::source::git::pkg_ref::GitPackageRef;
use crate::source::legacy_pesde::PesdeVersionedManifest;
use crate::source::path::RelativeOrAbsolutePath;
use crate::source::wally::compat_util::WALLY_MANIFEST_FILE_NAME;
use crate::source::wally::compat_util::get_exports;
use crate::source::wally::manifest::WallyManifest;
use crate::util::simplify_path;
use fs_err::tokio as fs;
use semver::BuildMetadata;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use tokio::task::JoinSet;
use tracing::instrument;

pub mod backend;
pub mod pkg_ref;
pub mod specifier;

/// State for Git package source
/// State for Git package source
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSourceState(());

/// The Git package source
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct GitPackageSource {
	repo: GitPackageBackends,
}
ser_display_deser_fromstr!(GitPackageSource);

impl Display for GitPackageSource {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.repo)
	}
}

impl FromStr for GitPackageSource {
	type Err = crate::source::git::backend::errors::ParseBackendError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self::new)
	}
}

impl GitPackageSource {
	/// Creates a new Git package source
	#[must_use]
	pub fn new(repo: GitPackageBackends) -> Self {
		Self { repo }
	}

	/// Creates a Git package source from a URL
	#[must_use]
	pub fn from_url(repo_url: GixUrl) -> Self {
		Self::new(GitPackageBackends::Git(GixPackageSourceBackend::new(
			repo_url,
		)))
	}

	/// Gets the repository backend
	#[must_use]
	pub fn repo(&self) -> &GitPackageBackends {
		&self.repo
	}
}

impl PackageSource for GitPackageSource {
	type RefreshError = errors::RefreshError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetExportsError = errors::GetExportsError;

	#[instrument(skip_all, level = "debug")]
	async fn refresh(
		&self,
		project: &Project,
		_old_state: Option<&SourceState>,
	) -> Result<SourceState, Self::RefreshError> {
		self.repo.refresh(project).await?;
		Ok(SourceState::Git(GitSourceState(())))
	}

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		subproject: &Subproject,
		_source_state: &SourceState,
		specifier: &DependencySpecifiers,
		_refreshed_sources: &RefreshedSources,
	) -> Result<ResolveResult, Self::ResolveError> {
		let DependencySpecifiers::Git(specifier) = specifier else {
			unreachable!("invalid specifier type for Git package source");
		};

		let tree_id = self
			.repo
			.resolve_rev(
				subproject.project(),
				specifier.rev.clone(),
				specifier.path.clone(),
			)
			.await
			.map_err(errors::ResolveErrorKind::ResolveRev)?;

		let tree_version = Version {
			major: 0,
			minor: 0,
			patch: 0,
			build: BuildMetadata::EMPTY,
			pre: tree_id.parse().unwrap(),
		};

		if let Some(manifest_bytes) = self
			.repo
			.read_file(
				subproject.project(),
				tree_id.clone(),
				MANIFEST_FILE_NAME.into(),
			)
			.await
			.map_err(errors::ResolveErrorKind::ReadManifest)?
		{
			let manifest =
				toml::from_str::<PesdeVersionedManifest>(&String::from_utf8_lossy(&manifest_bytes))
					.map_err(errors::ResolveErrorKind::DeserManifest)?;

			let structure_kind = match &manifest {
				PesdeVersionedManifest::Legacy(m) => StructureKind::LegacyPesde(m.target.kind()),
				PesdeVersionedManifest::Modern(_) => StructureKind::Pesde,
			};

			let dependencies = transform_pesde_dependencies(
				subproject,
				manifest.as_manifest(),
				self.repo.repo_url(),
			)?;

			return Ok(ResolveResult {
				source: PackageSources::Git(self.clone()),
				pkg_ref: PackageRefs::Git(GitPackageRef { tree_id }),
				structure_kind,
				versions: BTreeMap::from([(tree_version, dependencies)]),
			});
		}

		let manifest_bytes = self
			.repo
			.read_file(
				subproject.project(),
				tree_id.clone(),
				WALLY_MANIFEST_FILE_NAME.into(),
			)
			.await
			.map_err(errors::ResolveErrorKind::ReadManifest)?;

		let Some(manifest_bytes) = manifest_bytes else {
			return Err(errors::ResolveErrorKind::NoManifest(self.repo.repo_url().clone()).into());
		};

		let manifest = toml::from_str::<WallyManifest>(&String::from_utf8_lossy(&manifest_bytes))
			.map_err(errors::ResolveErrorKind::DeserManifest)?;
		let (package, dependencies) = manifest
			.into_resolve_entry()
			.map_err(errors::ResolveErrorKind::CollectDependencies)?;

		Ok(ResolveResult {
			source: PackageSources::Git(self.clone()),
			pkg_ref: PackageRefs::Git(GitPackageRef { tree_id }),
			structure_kind: StructureKind::Wally(package.name.name().into()),
			versions: BTreeMap::from([(tree_version, dependencies)]),
		})
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		_source_state: &SourceState,
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
			.join(&*pkg_ref.tree_id);

		match fs::read_to_string(&index_file).await {
			Ok(s) => {
				tracing::debug!(
					"using cached index file for package {}#{}",
					self.repo,
					pkg_ref.tree_id
				);
				reporter.report_done();

				return toml::from_str::<PackageFs>(&s).map_err(|e| {
					errors::DownloadErrorKind::DeserializeFile(self.repo.to_string(), e).into()
				});
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(errors::DownloadErrorKind::ReadIndex(e).into()),
		}

		let entries = self
			.repo
			.list_tree(project, pkg_ref.tree_id.clone())
			.await?;

		let is_wally = package.structure_kind.is_wally();
		let mut tasks = JoinSet::new();

		let mut fs_entries = BTreeMap::new();

		for entry in entries {
			let Some(name) = entry.path.file_name() else {
				continue;
			};

			if entry.is_dir {
				if IGNORED_DIRS.contains(&name) {
					continue;
				}

				fs_entries.insert(entry.path, FsEntry::Directory);
				continue;
			}

			if IGNORED_FILES.contains(&name) {
				continue;
			}

			if !is_wally && ADDITIONAL_FORBIDDEN_FILES.contains(&name) {
				tracing::debug!(
					"removing {name} from {}#{} at {} - using new structure",
					self.repo,
					pkg_ref.tree_id,
					entry.path
				);
				continue;
			}

			let project = project.clone();
			let repo = self.repo.clone();
			let tree_id = pkg_ref.tree_id.clone();

			tasks.spawn(async move {
				let contents = repo
					.read_file(&project, tree_id, entry.path.clone())
					.await
					.map_err(errors::DownloadErrorKind::ReadFile)?;

				let Some(contents) = contents else {
					return Err(errors::DownloadErrorKind::MissingFile(entry.path.clone()).into());
				};

				let (_, hash) = store_in_cas(project.cas_dir(), &*contents)
					.await
					.map_err(errors::DownloadErrorKind::WriteIndex)?;

				Ok::<_, errors::DownloadError>((entry.path, FsEntry::File(hash)))
			});
		}

		while let Some(res) = tasks.join_next().await {
			let (path, entry) = res.unwrap()?;
			fs_entries.insert(path, entry);
		}

		let fs = PackageFs::Cached(fs_entries);

		if let Some(parent) = index_file.parent() {
			fs::create_dir_all(parent)
				.await
				.map_err(errors::DownloadErrorKind::WriteIndex)?;
		}

		fs::write(
			&index_file,
			toml::to_string(&fs)
				.map_err(|e| errors::DownloadErrorKind::SerializeIndex(self.repo.to_string(), e))?,
		)
		.await
		.map_err(errors::DownloadErrorKind::WriteIndex)?;

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

fn transform_pesde_dependencies(
	subproject: &Subproject,
	manifest: &Manifest,
	repo_url: &GixUrl,
) -> Result<BTreeMap<Alias, (DependencySpecifiers, DependencyType)>, errors::ResolveError> {
	let dependencies = manifest
		.all_dependencies()
		.map_err(errors::ResolveErrorKind::CollectDependencies)?;

	dependencies
		.into_iter()
		.map(|(alias, (mut spec, ty))| {
			match &mut spec {
				DependencySpecifiers::Pesde(specifier) => {
					specifier.registry = manifest
						.urls
						.pesde_registries
						.get(&specifier.registry)
						.ok_or_else(|| {
							errors::ResolveErrorKind::PesdeRegistryNotFound(
								specifier.registry.clone(),
								repo_url.clone(),
							)
						})?
						.to_string();
				}
				DependencySpecifiers::LegacyPesde(specifier) => {
					specifier.index = manifest
						.urls
						.pesde_indices
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
						.urls
						.wally_indices
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
						// no-op, the path is relative and within the subproject, so it's allowed
					} else if std::env::var("PESDE_IMPURE_GIT_DEP_PATHS")
						.is_ok_and(|s| !s.is_empty())
					{
						// no-op, mostly useful for absolute paths, relative paths are UB
					} else {
						return Err(errors::ResolveErrorKind::Path(repo_url.clone()).into());
					}
				}
			}

			Ok((alias, (spec, ty)))
		})
		.collect()
}

/// Errors that can occur when interacting with the Git package source
pub mod errors {
	use crate::GixUrl;
	use crate::manifest::errors::AllDependenciesError;
	use relative_path::RelativePathBuf;
	use thiserror::Error;

	/// Errors that can occur when refreshing the Git package source
	pub type RefreshError = crate::source::git::backend::errors::RefreshError;

	/// Errors that can occur when downloading a package from a Git package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		/// An error occurred deserializing a file in the backend
		#[error("error deserializing file in backend {0}")]
		DeserializeFile(String, #[source] toml::de::Error),

		/// An error occurred getting Wally exports
		#[error("error getting wally exports")]
		WallyGetExports(#[from] crate::source::wally::compat_util::errors::GetExportsError),

		/// An error occurred reading a file from the backend
		#[error("error reading file from backend")]
		ReadFile(#[from] crate::source::git::backend::errors::ReadFileError),

		/// An error occurred listing tree from the backend
		#[error("error listing tree from backend")]
		ListTree(#[from] crate::source::git::backend::errors::ListTreeError),

		/// An error occurred reading the index file
		#[error("error reading index file")]
		ReadIndex(#[source] std::io::Error),

		/// An error occurred writing the index file
		#[error("error writing index file")]
		WriteIndex(#[source] std::io::Error),

		/// The file at the specified path was missing
		#[error("missing file at {0}")]
		MissingFile(RelativePathBuf),

		/// An error occurred serializing the index file for the backend
		#[error("error serializing the index file for backend {0}")]
		SerializeIndex(String, #[source] toml::ser::Error),
	}

	/// Errors that can occur when resolving a package from a Git package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ResolveError))]
	#[non_exhaustive]
	pub enum ResolveErrorKind {
		/// An error occurred resolving rev
		#[error("error resolving rev")]
		ResolveRev(#[from] crate::source::git::backend::errors::ResolveRevError),

		/// An error occurred reading the manifest
		#[error("error reading manifest")]
		ReadManifest(#[from] crate::source::git::backend::errors::ReadFileError),

		/// An error occurred deserializing a manifest
		#[error("error deserializing manifest")]
		DeserManifest(#[source] toml::de::Error),

		/// An error occurred collecting all manifest dependencies
		#[error("error collecting dependencies")]
		CollectDependencies(#[from] AllDependenciesError),

		/// No manifest was found in the backend
		#[error("no manifest found in backend {0}")]
		NoManifest(GixUrl),

		/// A pesde registry specified in the manifest was not found
		#[error("pesde registry {0} not found in {1}")]
		PesdeRegistryNotFound(String, GixUrl),

		/// A pesde index specified in the manifest was not found
		#[error("pesde index {0} not found in {1}")]
		PesdeIndexNotFound(String, GixUrl),

		/// A Wally index specified in the manifest was not found
		#[error("wally index {0} not found in {1}")]
		WallyIndexNotFound(String, GixUrl),

		/// The package depends on a path package that escapes the subproject
		#[error("path dependency in {0} is not allowed")]
		Path(GixUrl),
	}

	/// Errors that can occur when getting exports from a Git package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GetExportsError))]
	#[non_exhaustive]
	pub enum GetExportsErrorKind {
		/// An error occurred reading the manifest
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// An error occurred getting Wally exports
		#[error("error getting Wally exports")]
		WallyGetExports(#[from] crate::source::wally::compat_util::errors::GetExportsError),
	}
}
