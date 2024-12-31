use crate::{
    deser_manifest,
    manifest::target::Target,
    names::PackageNames,
    reporters::DownloadProgressReporter,
    source::{
        fs::PackageFS,
        ids::VersionId,
        path::pkg_ref::PathPackageRef,
        specifiers::DependencySpecifiers,
        traits::{DownloadOptions, PackageSource, ResolveOptions},
        ResolveResult,
    },
    DEFAULT_INDEX_NAME,
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
                            let index_name = spec.index.as_deref().unwrap_or(DEFAULT_INDEX_NAME);

                            spec.index = Some(
                                manifest
                                    .indices
                                    .get(index_name)
                                    .ok_or_else(|| {
                                        errors::ResolveError::IndexNotFound(
                                            index_name.to_string(),
                                            specifier.path.clone(),
                                        )
                                    })?
                                    .to_string(),
                            )
                        }
                        #[cfg(feature = "wally-compat")]
                        DependencySpecifiers::Wally(spec) => {
                            let index_name = spec.index.as_deref().unwrap_or(DEFAULT_INDEX_NAME);

                            spec.index = Some(
                                manifest
                                    .wally_indices
                                    .get(index_name)
                                    .ok_or_else(|| {
                                        errors::ResolveError::IndexNotFound(
                                            index_name.to_string(),
                                            specifier.path.clone(),
                                        )
                                    })?
                                    .to_string(),
                            )
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
        _options: &DownloadOptions<R>,
    ) -> Result<(PackageFS, Target), Self::DownloadError> {
        let manifest = deser_manifest(&pkg_ref.path).await?;

        Ok((
            PackageFS::Copy(pkg_ref.path.clone(), manifest.target.kind()),
            manifest.target,
        ))
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
}
