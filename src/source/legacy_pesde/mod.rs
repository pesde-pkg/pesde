//! Legacy pesde package source
#![deprecated = "pesde has redesigned its registries. See https://github.com/pesde-pkg/pesde/issues/69"]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fmt::Debug;
use std::fmt::Display;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use backend::GitLegacyPesdePackageSourceBackend;
use backend::IndexDependencySpecifiers;
use backend::IndexFile;
use backend::LegacyPesdePackageBackends;
use backend::LegacyPesdePackageSourceBackend as _;
use backend::VersionId;
use futures::StreamExt as _;
use pkg_ref::LegacyPesdePackageRef;
use serde::Deserialize;
use specifier::LegacyPesdeDependencySpecifier;

use crate::GixUrl;
use crate::Project;
use crate::RefreshedSources;
use crate::Subproject;
use crate::manifest::Manifest;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
use crate::source::DependencySpecifiers;
use crate::source::IGNORED_DIRS;
use crate::source::IGNORED_FILES;
use crate::source::PackageExports;
use crate::source::PackageRefs;
use crate::source::PackageSource;
use crate::source::PackageSources;
use crate::source::Realm;
use crate::source::ResolveResult;
use crate::source::ResolvedPackage;
use crate::source::StructureKind;
use crate::source::fs::FsEntry;
use crate::source::fs::PackageFs;
use crate::source::fs::store_in_cas;
use crate::source::git::specifier::GitDependencySpecifier;
use crate::source::legacy_pesde::target::Target;
use crate::source::wally::specifier::WallyDependencySpecifier;
use crate::util::ToEscaped as _;
use crate::version_matches;
use fs_err::tokio as fs;
use semver::Version;
use tracing::instrument;

pub mod backend;
pub mod pkg_ref;
pub mod specifier;
/// Targets
pub mod target;

/// The legacy pesde package source
#[derive(Debug, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct LegacyPesdePackageSource {
	repo: LegacyPesdePackageBackends,
}
ser_display_deser_fromstr!(LegacyPesdePackageSource);

impl Display for LegacyPesdePackageSource {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.repo)
	}
}

impl FromStr for LegacyPesdePackageSource {
	type Err = crate::source::legacy_pesde::backend::errors::ParseBackendError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self::new)
	}
}

impl LegacyPesdePackageSource {
	/// Creates a new legacy pesde package source
	#[must_use]
	pub fn new(repo: LegacyPesdePackageBackends) -> Self {
		Self { repo }
	}

	/// Creates a legacy pesde package source from a URL
	#[must_use]
	pub fn from_url(repo_url: GixUrl) -> Self {
		Self::new(LegacyPesdePackageBackends::Git(
			GitLegacyPesdePackageSourceBackend::new(repo_url),
		))
	}

	/// Gets the repository backend
	#[must_use]
	pub fn repo(&self) -> &LegacyPesdePackageBackends {
		&self.repo
	}
}

impl PackageSource for LegacyPesdePackageSource {
	type RefreshError = errors::RefreshError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetExportsError = errors::GetExportsError;

	#[instrument(skip_all, level = "debug")]
	async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
		self.repo.refresh(project).await
	}

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		subproject: &Subproject,
		specifier: &DependencySpecifiers,
		_refreshed_sources: &RefreshedSources,
	) -> Result<ResolveResult, Self::ResolveError> {
		let DependencySpecifiers::LegacyPesde(specifier) = specifier else {
			unreachable!("invalid specifier type for legacy pesde package source");
		};

		let Some(IndexFile { entries, .. }) = self
			.repo
			.read_index_file(subproject.project(), specifier.name.clone())
			.await?
		else {
			return Err(errors::ResolveErrorKind::NotFound(specifier.name.clone()).into());
		};

		tracing::debug!("{} has {} possible entries", specifier.name, entries.len());

		let mut suggestions = BTreeSet::new();

		let versions = entries
			.into_iter()
			.filter(|(_, entry)| !entry.yanked)
			.filter(|(v_id, _)| version_matches(&specifier.version, v_id.version()))
			.inspect(|(v_id, _)| {
				suggestions.insert(v_id.target());
			})
			.filter(|(v_id, _)| specifier.target == v_id.target())
			.map(|(v_id, entry)| {
				(
					v_id.into_version(),
					entry
						.dependencies
						.into_iter()
						.map(|(alias, (specifiers, dep_type))| {
							(
								alias,
								(
									match specifiers {
										IndexDependencySpecifiers::LegacyPesde(s) => {
											DependencySpecifiers::LegacyPesde(
												LegacyPesdeDependencySpecifier {
													name: s.name,
													version: s.version,
													index: s.index,
													target: s.target.unwrap_or(entry.target.kind()),
												},
											)
										}
										IndexDependencySpecifiers::Wally(s) => {
											DependencySpecifiers::Wally(WallyDependencySpecifier {
												name: s.name,
												version: s.version,
												index: s.index,
												// TODO: query WallyPackageSource for realm based on the package's canonical value (WallyPackage.realm)
												realm: Realm::Shared,
											})
										}
										IndexDependencySpecifiers::Git(s) => {
											DependencySpecifiers::Git(GitDependencySpecifier {
												repo: s.repo,
												rev: s.rev,
												path: s.path,
												// no easy way to get this data, probably not worth it since this compat code is temporary
												realm: None,
											})
										}
									},
									dep_type,
								),
							)
						})
						.collect(),
				)
			})
			.collect::<BTreeMap<_, _>>();

		if versions.is_empty() {
			return Err(errors::ResolveErrorKind::NoMatchingVersion(
				specifier.clone(),
				specifier.target,
				suggestions,
			)
			.into());
		}

		Ok(ResolveResult {
			source: PackageSources::LegacyPesde(self.clone()),
			pkg_ref: PackageRefs::LegacyPesde(LegacyPesdePackageRef {
				name: specifier.name.clone(),
				target: specifier.target,
			}),
			structure_kind: StructureKind::LegacyPesde(specifier.target),
			versions,
		})
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter + 'static>(
		&self,
		project: &Project,
		package: &ResolvedPackage,
		reporter: Arc<R>,
	) -> Result<PackageFs, Self::DownloadError> {
		let PackageRefs::LegacyPesde(pkg_ref) = package.id.pkg_ref() else {
			unreachable!("invalid package ref type for pesde package source");
		};

		let index_file = project
			.cas_dir()
			.join("index")
			.join("pesde")
			.join(self.repo.to_string().escaped())
			.join(pkg_ref.name.scope().to_string().escaped())
			.join(pkg_ref.name.name().to_string().escaped())
			.join(package.id.version().to_string())
			.join(pkg_ref.target.to_string());

		match fs::read_to_string(&index_file).await {
			Ok(s) => {
				tracing::debug!(
					"using cached index file for package {}@{} {}",
					pkg_ref.name,
					package.id.version(),
					pkg_ref.target
				);

				reporter.report_done();

				return toml::from_str::<PackageFs>(&s).map_err(Into::into);
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(errors::DownloadErrorKind::ReadIndex(e).into()),
		}

		let version_id = VersionId::new(package.id.version().clone(), pkg_ref.target);
		let entries_stream = self
			.repo
			.download_entries(project, &pkg_ref.name, &version_id, reporter)
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
		package: &ResolvedPackage,
		_path: &Path,
	) -> Result<PackageExports, Self::GetExportsError> {
		let PackageRefs::LegacyPesde(pkg_ref) = package.id.pkg_ref() else {
			unreachable!("invalid package ref type for legacy pesde package source");
		};

		let Some(IndexFile { mut entries, .. }) = self
			.repo
			.read_index_file(project, pkg_ref.name.clone())
			.await?
		else {
			return Err(errors::GetExportsErrorKind::NotFound(pkg_ref.name.clone()).into());
		};

		let entry = entries
			.remove(&VersionId::new(
				package.id.version().clone(),
				pkg_ref.target,
			))
			.ok_or_else(|| errors::GetExportsErrorKind::NotFound(pkg_ref.name.clone()))?;

		Ok(entry.target.into_exports())
	}
}

/// A legacy pesde (<0.8) manifest
#[derive(Debug, Deserialize)]
pub struct LegacyPesdeManifest {
	/// The version
	pub version: Version,
	/// The target
	pub target: Target,
	/// The modern pesde compatible fields
	#[serde(flatten)]
	pub manifest: Manifest,
	/// Any extra fields
	#[serde(flatten)]
	pub user_defined_fields: HashMap<String, toml::Value>,
}

/// A manifest for either version of pesde
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum PesdeVersionedManifest {
	/// [Manifest]
	Modern(Manifest),
	/// [LegacyPesdeManifest]
	Legacy(LegacyPesdeManifest),
}

impl PesdeVersionedManifest {
	/// Returns the manifest
	#[must_use]
	pub fn as_manifest(&self) -> &Manifest {
		match self {
			Self::Legacy(m) => &m.manifest,
			Self::Modern(m) => m,
		}
	}

	/// Returns the manifest
	#[must_use]
	pub fn into_manifest(self) -> Manifest {
		match self {
			Self::Legacy(m) => m.manifest,
			Self::Modern(m) => m,
		}
	}

	/// Returns the exports for this manifest
	#[must_use]
	pub fn as_exports(&self) -> PackageExports {
		match self {
			Self::Legacy(m) => m.target.clone().into_exports(),
			Self::Modern(m) => m.as_exports(),
		}
	}
}

/// Errors that can occur when interacting with the legacy pesde package source
pub mod errors {
	use std::collections::BTreeSet;

	use itertools::Itertools as _;
	use thiserror::Error;

	use super::backend::errors::ReadIndexFileError;
	use super::target::TargetKind;
	use crate::names::PackageName;
	use crate::source::legacy_pesde::specifier::LegacyPesdeDependencySpecifier;

	pub use super::backend::errors::RefreshError;
	pub use super::backend::errors::VersionIdParseError;

	/// Errors that can occur when resolving a package from a legacy pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ResolveError))]
	#[non_exhaustive]
	pub enum ResolveErrorKind {
		/// Package not found in index
		#[error("package `{0}` not found")]
		NotFound(PackageName),

		// custom error to provide the user with target suggestions
		/// No matching version was found for a specifier
		#[error("no matching version found for {0} {1}. available targets: {suggestions}", suggestions = .2.iter().format(", "))]
		NoMatchingVersion(
			LegacyPesdeDependencySpecifier,
			TargetKind,
			BTreeSet<TargetKind>,
		),

		/// Error reading index file
		#[error("error reading index file")]
		ReadIndex(#[from] ReadIndexFileError),
	}

	/// Errors that can occur when downloading a package from a legacy pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		/// Error from backend
		#[error("error from backend")]
		Backend(#[from] crate::source::legacy_pesde::backend::errors::DownloadError),

		/// Error reading index file
		#[error("error reading index file")]
		ReadIndex(#[source] std::io::Error),

		/// Error writing index file
		#[error("error writing index file")]
		WriteIndex(#[source] std::io::Error),

		/// Error serializing index file
		#[error("error serializing index file")]
		SerializeIndex(#[from] toml::ser::Error),

		/// Error deserializing index file
		#[error("error deserializing index file")]
		DeserializeIndex(#[from] toml::de::Error),
	}

	/// Errors that can occur when getting the target for a package from a legacy pesde package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GetExportsError))]
	#[non_exhaustive]
	pub enum GetExportsErrorKind {
		/// Error reading index file
		#[error("error reading index file")]
		ReadIndex(#[from] ReadIndexFileError),

		/// Package not found in index
		#[error("package `{0}` not found in index")]
		NotFound(PackageName),
	}
}
