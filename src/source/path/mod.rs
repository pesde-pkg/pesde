#![expect(deprecated)]
use crate::{
	deser_manifest,
	manifest::target::Target,
	reporters::DownloadProgressReporter,
	source::{
		PackageSources, ResolveResult,
		fs::PackageFs,
		ids::{PackageId, VersionId},
		path::pkg_ref::PathPackageRef,
		refs::{PackageRefs, ResolveRecord},
		specifiers::DependencySpecifiers,
		traits::{DownloadOptions, GetTargetOptions, PackageSource, ResolveOptions},
	},
};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tracing::instrument;

/// The path package reference
pub mod pkg_ref;
/// The path dependency specifier
pub mod specifier;

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
		_options: &ResolveOptions,
	) -> Result<ResolveResult<Self::Ref>, Self::ResolveError> {
		// let ResolveOptions { project, .. } = options;

		let manifest = deser_manifest(&specifier.path).await?;

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
							.get(&spec.index)
							.ok_or_else(|| {
								errors::ResolveError::IndexNotFound(
									spec.index.clone(),
									specifier.path.clone(),
								)
							})?
							.to_string();
					}
					#[cfg(feature = "wally-compat")]
					DependencySpecifiers::Wally(spec) => {
						spec.index = manifest
							.wally_indices
							.get(&spec.index)
							.ok_or_else(|| {
								errors::ResolveError::IndexNotFound(
									spec.index.clone(),
									specifier.path.clone(),
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

		let pkg_ref = PathPackageRef {
			path: specifier.path.clone(),
		};

		Ok((
			BTreeMap::from([(
				PackageId::new(
					PackageSources::Path(*self),
					PackageRefs::Path(pkg_ref.clone()),
					VersionId::new(
						/* TODO */ Version::new(0, 1, 0),
						manifest.target.kind(),
					),
				),
				ResolveRecord {
					pkg_ref,
					dependencies,
				},
			)]),
			BTreeSet::new(),
		))
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter>(
		&self,
		pkg_ref: &Self::Ref,
		options: &DownloadOptions<R>,
	) -> Result<PackageFs, Self::DownloadError> {
		let DownloadOptions { reporter, .. } = options;
		let manifest = deser_manifest(&pkg_ref.path).await?;

		reporter.report_done();

		Ok(PackageFs::Copy(
			pkg_ref.path.clone(),
			manifest.target.kind(),
		))
	}

	#[instrument(skip_all, level = "debug")]
	async fn get_target(
		&self,
		pkg_ref: &Self::Ref,
		_options: &GetTargetOptions,
	) -> Result<Target, Self::GetTargetError> {
		let manifest = deser_manifest(&pkg_ref.path).await?;

		Ok(manifest.target)
	}
}

/// Errors that can occur when using a path package source
pub mod errors {
	use crate::{manifest::target::TargetKind, names::PackageName};
	use std::path::PathBuf;
	use thiserror::Error;

	/// Errors that can occur when refreshing the path package source
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum RefreshError {}

	/// Errors that can occur when resolving a path package
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum ResolveError {
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

		/// Finding workspace members failed
		#[error("failed to find workspace members")]
		WorkspaceMembers(#[from] crate::errors::WorkspaceMembersError),

		/// Workspace package not found
		#[error("workspace package {0} {1} not found in package")]
		WorkspacePackageNotFound(PackageName, TargetKind),
	}

	/// Errors that can occur when downloading a path package
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum DownloadError {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),
	}

	/// Errors that can occur when getting the target of a path package
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum GetTargetError {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),
	}
}
