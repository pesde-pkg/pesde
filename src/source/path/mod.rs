use crate::{
	deser_manifest,
	manifest::target::Target,
	names::PackageNames,
	reporters::DownloadProgressReporter,
	source::{
		fs::PackageFs,
		ids::VersionId,
		path::pkg_ref::PathPackageRef,
		specifiers::DependencySpecifiers,
		traits::{DownloadOptions, GetTargetOptions, PackageSource, ResolveOptions},
		ResolveResult,
	},
};
use std::collections::BTreeMap;
use tracing::instrument;

/// The path package reference
pub mod pkg_ref;
/// The path dependency specifier
pub mod specifier;

/// The path package source
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
		let manifest = deser_manifest(&specifier.path).await?;

		let pkg_ref = PathPackageRef {
			path: specifier.path.clone(),
			dependencies: manifest
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
						DependencySpecifiers::Workspace(_) => {}
						DependencySpecifiers::Path(_) => {}
					}

					Ok((alias, (spec, ty)))
				})
				.collect::<Result<_, errors::ResolveError>>()?,
		};

		Ok((
			PackageNames::Pesde(manifest.name),
			BTreeMap::from([(
				VersionId::new(manifest.version, manifest.target.kind()),
				pkg_ref,
			)]),
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
