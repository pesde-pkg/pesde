//! Wally package source
use crate::GixUrl;
use crate::Project;
use crate::RefreshedSources;
use crate::Subproject;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
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
use crate::source::wally::backend::GitWallyPackageSourceBackend;
use crate::source::wally::backend::WallyPackageBackends;
use crate::source::wally::backend::WallyPackageSourceBackend as _;
use crate::source::wally::compat_util::get_exports;
use crate::source::wally::manifest::WallyManifest;
use crate::source::wally::pkg_ref::WallyPackageRef;
use crate::util::ToEscaped as _;
use crate::version_matches;
use fs_err::tokio as fs;
use futures::StreamExt as _;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use tracing::instrument;

pub mod backend;
pub(crate) mod compat_util;
pub(crate) mod manifest;
pub mod pkg_ref;
pub mod specifier;

/// State for Wally package source
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WallySourceState(());

/// The Wally package source
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct WallyPackageSource {
	repo: WallyPackageBackends,
}
ser_display_deser_fromstr!(WallyPackageSource);

impl Display for WallyPackageSource {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.repo)
	}
}

impl FromStr for WallyPackageSource {
	type Err = crate::source::wally::backend::errors::ParseBackendError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self::new)
	}
}

impl WallyPackageSource {
	/// Creates a new Wally package source
	#[must_use]
	pub fn new(repo: WallyPackageBackends) -> Self {
		Self { repo }
	}

	/// Creates a Wally package source from a URL
	#[must_use]
	pub fn from_url(repo_url: GixUrl) -> Self {
		Self::new(WallyPackageBackends::Git(
			GitWallyPackageSourceBackend::new(repo_url),
		))
	}

	/// Gets the repository backend
	#[must_use]
	pub fn repo(&self) -> &WallyPackageBackends {
		&self.repo
	}
}

impl PackageSource for WallyPackageSource {
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
		Ok(SourceState::Wally(WallySourceState(())))
	}

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		subproject: &Subproject,
		_source_state: &SourceState,
		specifier: &DependencySpecifiers,
		refreshed_sources: &RefreshedSources,
	) -> Result<ResolveResult, Self::ResolveError> {
		let DependencySpecifiers::Wally(specifier) = specifier else {
			unreachable!("invalid specifier type for Wally package source");
		};

		let mut string = self
			.repo
			.read_index_file(subproject.project(), specifier.name.clone())
			.await?;
		let mut repo = self.repo.clone();

		if string.is_none() {
			tracing::debug!(
				"{} not found in Wally registry. searching in backup registries",
				specifier.name
			);
			let config = self.repo.config(subproject.project()).await?;

			for fallback_repo in config.fallback_registries {
				match refreshed_sources
					.refresh(
						&PackageSources::Wally(WallyPackageSource::new(fallback_repo.clone())),
						subproject.project(),
						None,
					)
					.await
					.map_err(super::errors::RefreshError::into_inner)
				{
					Ok(_) => {}
					Err(super::errors::RefreshErrorKind::Wally(e)) => {
						return Err(errors::ResolveErrorKind::Refresh(e).into());
					}
					Err(e) => panic!("unexpected error: {e:?}"),
				}

				match fallback_repo
					.read_index_file(subproject.project(), specifier.name.clone())
					.await
				{
					Ok(Some(res)) => {
						string = Some(res);
						repo = fallback_repo;
						break;
					}
					Ok(None) => {
						tracing::debug!("{} not found in {}", specifier.name, fallback_repo);
					}
					Err(e) => return Err(errors::ResolveErrorKind::ReadIndex(e).into()),
				}
			}
		}

		let Some(string) = string else {
			return Err(errors::ResolveErrorKind::NotFound(specifier.name.clone()).into());
		};

		let entries: Vec<WallyManifest> = string
			.lines()
			.map(serde_json::from_str)
			.collect::<Result<_, _>>()
			.map_err(|e| errors::ResolveErrorKind::Parse(specifier.name.clone(), e))?;

		tracing::debug!("{} has {} possible entries", specifier.name, entries.len());

		let versions = entries
			.into_iter()
			.filter(|manifest| version_matches(&specifier.version, &manifest.package.version))
			.map(|mut manifest| {
				// ensure a consistent registry value to improve deduplication
				#[expect(irrefutable_let_patterns)]
				if let WallyPackageBackends::Git(repo) = &repo {
					manifest.package.registry = repo.repo_url().clone();
				}

				manifest
					.into_resolve_entry()
					.map(|(package, deps)| (package.version, deps))
					.map_err(|e| {
						errors::ResolveErrorKind::AllDependencies(specifier.name.clone(), e).into()
					})
			})
			.collect::<Result<_, errors::ResolveError>>()?;

		Ok(ResolveResult {
			source: PackageSources::Wally(WallyPackageSource { repo }),
			pkg_ref: PackageRefs::Wally(WallyPackageRef {
				name: specifier.name.clone(),
			}),
			structure_kind: StructureKind::Wally(specifier.name.name().into()),
			versions,
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
		let PackageRefs::Wally(pkg_ref) = package.id.pkg_ref() else {
			unreachable!("invalid package ref type for Wally package source");
		};

		let index_file = project
			.cas_dir()
			.join("index")
			.join("wally")
			.join(self.repo.to_string().escaped())
			.join(pkg_ref.name.scope().to_string().escaped())
			.join(pkg_ref.name.name().to_string().escaped())
			.join(package.id.version().to_string());

		match fs::read_to_string(&index_file).await {
			Ok(s) => {
				tracing::debug!(
					"using cached index file for package {}@{}",
					pkg_ref.name,
					package.id.version()
				);

				reporter.report_done();

				return toml::from_str::<PackageFs>(&s).map_err(Into::into);
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(errors::DownloadErrorKind::ReadIndex(e).into()),
		}

		let entries_stream = self
			.repo
			.download_entries(project, &pkg_ref.name, package.id.version(), reporter)
			.await?;
		tokio::pin!(entries_stream);

		let mut entries = BTreeMap::new();

		while let Some(entry_result) = entries_stream.next().await {
			let (path, contents) = entry_result?;
			let Some(name) = path.file_name() else {
				continue;
			};

			let Some(contents) = contents else {
				if IGNORED_DIRS.contains(&name) {
					continue;
				}
				entries.insert(path, FsEntry::Directory);
				continue;
			};

			if IGNORED_FILES.contains(&name) {
				continue;
			}

			let (_, hash) = store_in_cas(project.cas_dir(), &*contents)
				.await
				.map_err(errors::DownloadErrorKind::WriteIndex)?;

			entries.insert(path, FsEntry::File(hash));
		}

		let fs = PackageFs::Cached(entries);

		if let Some(parent) = index_file.parent() {
			fs::create_dir_all(parent)
				.await
				.map_err(errors::DownloadErrorKind::WriteIndex)?;
		}

		fs::write(&index_file, toml::to_string(&fs)?)
			.await
			.map_err(errors::DownloadErrorKind::WriteIndex)?;

		Ok(fs)
	}

	#[instrument(skip_all, level = "debug")]
	async fn get_exports(
		&self,
		project: &Project,
		_package: &ResolvedPackage,
		path: &Path,
	) -> Result<PackageExports, Self::GetExportsError> {
		get_exports(project, path).await.map_err(Into::into)
	}
}

/// Errors that can occur when interacting with a Wally package source
pub mod errors {
	use crate::names::WallyPackageName;
	use thiserror::Error;

	/// Errors that can occur when refreshing the Wally package source
	pub type RefreshError = crate::source::wally::backend::errors::RefreshError;

	/// Errors that can occur when resolving a package from a Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ResolveError))]
	#[non_exhaustive]
	pub enum ResolveErrorKind {
		/// Package not found in index
		#[error("package {0} not found")]
		NotFound(WallyPackageName),

		/// Error parsing file for package
		#[error("error parsing file for {0}")]
		Parse(WallyPackageName, #[source] serde_json::Error),

		/// Error parsing all dependencies
		#[error("error parsing all dependencies for {0}")]
		AllDependencies(
			WallyPackageName,
			#[source] crate::manifest::errors::AllDependenciesError,
		),

		/// Error from backend
		#[error("error from backend")]
		Backend(#[from] crate::source::wally::backend::errors::ConfigError),

		/// Error reading index file
		#[error("error reading index file")]
		ReadIndex(#[from] crate::source::wally::backend::errors::ReadIndexFileError),

		/// Error refreshing source
		#[error("error refreshing source")]
		Refresh(#[from] crate::source::wally::backend::errors::RefreshError),
	}

	/// Errors that can occur when downloading a package from a Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = WallyDownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		/// Error from backend
		#[error("error from backend")]
		Backend(#[from] crate::source::wally::backend::errors::DownloadError),

		/// Error deserializing index file
		#[error("error deserializing index file")]
		Deserialize(#[from] toml::de::Error),

		/// Error reading index file
		#[error("error reading index file")]
		ReadIndex(#[source] std::io::Error),

		/// Error decompressing archive
		#[error("error decompressing archive")]
		Decompress(#[from] async_zip::error::ZipError),

		/// Error serializing index file
		#[error("error serializing index file")]
		SerializeIndex(#[from] toml::ser::Error),

		/// Error getting package exports
		#[error("error getting package exports")]
		GetExports(#[from] crate::source::wally::compat_util::errors::GetExportsError),

		/// Error writing index file
		#[error("error writing index file")]
		WriteIndex(#[source] std::io::Error),
	}

	/// The error type for downloading a package from a Wally package source
	pub type DownloadError = WallyDownloadError;

	/// Errors that can occur when getting exports from a Wally package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GetExportsError))]
	#[non_exhaustive]
	pub enum GetExportsErrorKind {
		/// Error getting package exports
		#[error("error getting package exports")]
		GetExports(#[from] crate::source::wally::compat_util::errors::GetExportsError),
	}
}
