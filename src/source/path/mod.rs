//! Path package source
use crate::MANIFEST_FILE_NAME;
use crate::Project;
use crate::RefreshedSources;
use crate::Subproject;
use crate::errors::ManifestReadError;
use crate::errors::ManifestReadErrorKind;
use crate::manifest::Manifest;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
use crate::source::DependencySpecifiers;
use crate::source::PackageExports;
use crate::source::PackageRefs;
use crate::source::PackageSource;
use crate::source::PackageSources;
use crate::source::ResolveResult;
use crate::source::ResolvedPackage;
use crate::source::StructureKind;
use crate::source::fs::PackageFs;
use crate::source::path::pkg_ref::PathPackageRef;
use fs_err::tokio as fs;
use relative_path::RelativePathBuf;
use semver::BuildMetadata;
use semver::Prerelease;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tracing::instrument;

pub mod pkg_ref;
pub mod specifier;

pub(crate) fn local_version() -> Version {
	Version {
		major: 0,
		minor: 0,
		patch: 0,
		pre: Prerelease::new("pesde").unwrap(),
		build: BuildMetadata::EMPTY,
	}
}

/// The path for a path dependency specifier
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelativeOrAbsolutePath {
	/// A relative path
	Relative(RelativePathBuf),
	/// An absolute path
	Absolute(PathBuf),
}
ser_display_deser_fromstr!(RelativeOrAbsolutePath);

impl Display for RelativeOrAbsolutePath {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			RelativeOrAbsolutePath::Relative(rel_path) => write!(f, "{}", rel_path.display()),
			RelativeOrAbsolutePath::Absolute(abs_path) => write!(f, "{}", abs_path.display()),
		}
	}
}

impl FromStr for RelativeOrAbsolutePath {
	type Err = std::convert::Infallible;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(RelativePathBuf::from_path(s).map_or_else(
			|_| RelativeOrAbsolutePath::Absolute(PathBuf::from(s)),
			RelativeOrAbsolutePath::Relative,
		))
	}
}

/// The path package source
#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub struct PathPackageSource;

impl PackageSource for PathPackageSource {
	type RefreshError = errors::RefreshError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetExportsError = errors::GetExportsError;

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		subproject: &Subproject,
		specifier: &DependencySpecifiers,
		_refreshed_sources: &RefreshedSources,
	) -> Result<ResolveResult, Self::ResolveError> {
		let DependencySpecifiers::Path(specifier) = specifier else {
			unreachable!("invalid specifier type for path package source");
		};

		let path = match &specifier.path {
			RelativeOrAbsolutePath::Relative(rel_path) => rel_path.to_path(subproject.dir()),
			RelativeOrAbsolutePath::Absolute(abs_path) => abs_path.clone(),
		};

		let manifest = fs::read_to_string(path.join(MANIFEST_FILE_NAME))
			.await
			.map_err(|e| {
				errors::ResolveErrorKind::ManifestRead(
					crate::errors::ManifestReadErrorKind::Io(e).into(),
				)
			})?;
		let manifest: Manifest = toml::from_str(&manifest).map_err(|e| {
			errors::ResolveErrorKind::ManifestRead(
				crate::errors::ManifestReadErrorKind::Serde(path.clone(), e).into(),
			)
		})?;

		let dependencies = manifest
			.all_dependencies()?
			.into_iter()
			.map(|(alias, (mut spec, ty))| {
				match &mut spec {
					DependencySpecifiers::Pesde(spec) => {
						spec.index = manifest
							.indices
							.pesde
							.get(&spec.index)
							.ok_or_else(|| {
								errors::ResolveErrorKind::IndexNotFound(
									spec.index.clone(),
									path.clone(),
								)
							})?
							.to_string();
					}
					DependencySpecifiers::Wally(spec) => {
						spec.index = manifest
							.indices
							.wally
							.get(&spec.index)
							.ok_or_else(|| {
								errors::ResolveErrorKind::IndexNotFound(
									spec.index.clone(),
									path.clone(),
								)
							})?
							.to_string();
					}
					DependencySpecifiers::Git(_) => {}
					DependencySpecifiers::Path(_) => {}
				}

				Ok((alias, (spec, ty)))
			})
			.collect::<Result<_, errors::ResolveError>>()?;

		Ok(ResolveResult {
			source: PackageSources::Path(*self),
			pkg_ref: PackageRefs::Path(PathPackageRef {
				path: if let Ok(path) = path.strip_prefix(subproject.project().dir()) {
					path.to_path_buf()
				} else {
					path.clone()
				},
			}),
			structure_kind: StructureKind::PesdeV2,
			versions: BTreeMap::from([(local_version(), dependencies)]),
		})
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter>(
		&self,
		project: &Project,
		package: &ResolvedPackage,
		reporter: Arc<R>,
	) -> Result<PackageFs, Self::DownloadError> {
		let PackageRefs::Path(pkg_ref) = package.id.pkg_ref() else {
			unreachable!("invalid package ref type for path package source");
		};

		reporter.report_done();

		Ok(PackageFs::Copy(if pkg_ref.path.is_absolute() {
			pkg_ref.path.clone()
		} else {
			project.dir().join(&pkg_ref.path)
		}))
	}

	#[instrument(skip_all, level = "debug")]
	async fn get_exports(
		&self,
		_project: &Project,
		_package: &ResolvedPackage,
		path: &Path,
	) -> Result<PackageExports, Self::GetExportsError> {
		let manifest = fs::read_to_string(path.join(MANIFEST_FILE_NAME))
			.await
			.map_err(|e| ManifestReadError::from(ManifestReadErrorKind::Io(e)))?;
		let manifest: Manifest = toml::from_str(&manifest).map_err(|e| {
			ManifestReadError::from(ManifestReadErrorKind::Serde(path.to_path_buf(), e))
		})?;

		Ok(manifest.as_exports())
	}
}

/// Errors that can occur when using a path package source
pub mod errors {
	use std::path::PathBuf;
	use thiserror::Error;

	/// Errors that can occur when refreshing the path package source
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = RefreshError))]
	#[non_exhaustive]
	pub enum RefreshErrorKind {}

	/// Errors that can occur when resolving a path package
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ResolveError))]
	#[non_exhaustive]
	pub enum ResolveErrorKind {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// An error occurred getting all dependencies
		#[error("failed to get all dependencies")]
		AllDependencies(#[from] crate::manifest::errors::AllDependenciesError),

		/// An index of the package was not found
		#[error("index {0} not found in package {1}")]
		IndexNotFound(String, PathBuf),
	}

	/// Errors that can occur when downloading a path package
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = DownloadError))]
	#[non_exhaustive]
	pub enum DownloadErrorKind {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),
	}

	/// Errors that can occur when getting the target of a path package
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GetExportsError))]
	#[non_exhaustive]
	pub enum GetExportsErrorKind {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),
	}
}
