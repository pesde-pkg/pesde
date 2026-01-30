#![expect(deprecated)]
use crate::MANIFEST_FILE_NAME;
use crate::errors::ManifestReadError;
use crate::errors::ManifestReadErrorKind;
use crate::manifest::Manifest;
use crate::manifest::target::Target;
use crate::reporters::DownloadProgressReporter;
use crate::ser_display_deser_fromstr;
use crate::source::DependencySpecifiers;
use crate::source::PackageDependencies;
use crate::source::PackageRefs;
use crate::source::PackageSources;
use crate::source::fs::PackageFs;
use crate::source::ids::VersionId;
use crate::source::path::pkg_ref::PathPackageRef;
use crate::source::traits::DownloadOptions;
use crate::source::traits::GetTargetOptions;
use crate::source::traits::PackageSource;
use crate::source::traits::ResolveOptions;
use fs_err::tokio as fs;
use relative_path::RelativePathBuf;
use semver::BuildMetadata;
use semver::Prerelease;
use semver::Version;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;
use tracing::instrument;

/// The path package reference
pub mod pkg_ref;
/// The path dependency specifier
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
	type Specifier = specifier::PathDependencySpecifier;
	type Ref = PathPackageRef;
	type RefreshError = errors::RefreshError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetTargetError = errors::GetTargetError;

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		specifier: &Self::Specifier,
		options: &ResolveOptions,
	) -> Result<
		(
			PackageSources,
			PackageRefs,
			BTreeMap<VersionId, PackageDependencies>,
		),
		Self::ResolveError,
	> {
		let ResolveOptions { subproject, .. } = options;

		let path = match &specifier.path {
			RelativeOrAbsolutePath::Relative(rel_path) => {
				rel_path.to_path(subproject.project().dir())
			}
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

		// let (path_package_dir, path_workspace_dir) = find_roots(specifier.path.clone()).await?;
		// let path_project = Project::new(
		// 	path_package_dir,
		// 	path_workspace_dir,
		// 	// these don't matter, we're not using any functionality which uses them
		// 	project.data_dir(),
		// 	project.cas_dir(),
		// 	project.auth_config().clone(),
		// );

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

		Ok((
			PackageSources::Path(*self),
			PackageRefs::Path(PathPackageRef {
				path: specifier.path.clone(),
				absolute_path: path,
			}),
			BTreeMap::from([(
				VersionId::new(local_version(), manifest.target.kind()),
				PackageDependencies::Immediate(dependencies),
			)]),
		))
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter>(
		&self,
		pkg_ref: &Self::Ref,
		options: &DownloadOptions<'_, R>,
	) -> Result<PackageFs, Self::DownloadError> {
		let DownloadOptions { reporter, .. } = options;

		reporter.report_done();

		// safety: path packages are always resolved freshly by the resolver, so the path is always set to a proper value
		Ok(PackageFs::Copy(pkg_ref.absolute_path.clone()))
	}

	#[instrument(skip_all, level = "debug")]
	async fn get_target(
		&self,
		_pkg_ref: &Self::Ref,
		options: &GetTargetOptions<'_>,
	) -> Result<Target, Self::GetTargetError> {
		let GetTargetOptions { path, .. } = options;

		let manifest = fs::read_to_string(path.join(MANIFEST_FILE_NAME))
			.await
			.map_err(|e| ManifestReadError::from(ManifestReadErrorKind::Io(e)))?;
		let manifest: Manifest = toml::from_str(&manifest).map_err(|e| {
			ManifestReadError::from(ManifestReadErrorKind::Serde(path.to_path_buf(), e))
		})?;

		Ok(manifest.target)
	}
}

/// Errors that can occur when using a path package source
pub mod errors {
	use crate::manifest::target::TargetKind;
	use crate::names::PackageName;
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

		/// Finding the package roots failed
		#[error("failed to find package roots")]
		FindRoots(#[from] crate::errors::FindRootsError),

		/// Workspace package not found
		#[error("workspace package {0} {1} not found in package")]
		WorkspacePackageNotFound(PackageName, TargetKind),
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
	#[thiserror_ext(newtype(name = GetTargetError))]
	#[non_exhaustive]
	pub enum GetTargetErrorKind {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),
	}
}
