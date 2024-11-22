use crate::{
    manifest::target::{Target, TargetKind},
    names::PackageNames,
    source::{
        fs::PackageFS, refs::PackageRefs, specifiers::DependencySpecifiers, traits::*,
        version_id::VersionId,
    },
    Project,
};
use std::{
    collections::{BTreeMap, HashSet},
    fmt::Debug,
};

/// Packages' filesystems
pub mod fs;
/// The Git package source
pub mod git;
/// Git index-based package source utilities
pub mod git_index;
/// The pesde package source
pub mod pesde;
/// Package references
pub mod refs;
/// Dependency specifiers
pub mod specifiers;
/// Traits for sources and packages
pub mod traits;
/// Version IDs
pub mod version_id;
/// The Wally package source
#[cfg(feature = "wally-compat")]
pub mod wally;
/// The workspace package source
pub mod workspace;

/// Files that will not be stored when downloading a package. These are only files which break pesde's functionality, or are meaningless and possibly heavy (e.g. `.DS_Store`)
pub const IGNORED_FILES: &[&str] = &["foreman.toml", "aftman.toml", "rokit.toml", ".DS_Store"];

/// Directories that will not be stored when downloading a package. These are only directories which break pesde's functionality, or are meaningless and possibly heavy
pub const IGNORED_DIRS: &[&str] = &[".git"];

/// The result of resolving a package
pub type ResolveResult<Ref> = (PackageNames, BTreeMap<VersionId, Ref>);

/// All possible package sources
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub enum PackageSources {
    /// A pesde package source
    Pesde(pesde::PesdePackageSource),
    /// A Wally package source
    #[cfg(feature = "wally-compat")]
    Wally(wally::WallyPackageSource),
    /// A Git package source
    Git(git::GitPackageSource),
    /// A workspace package source
    Workspace(workspace::WorkspacePackageSource),
}

impl PackageSource for PackageSources {
    type Specifier = DependencySpecifiers;
    type Ref = PackageRefs;
    type RefreshError = errors::RefreshError;
    type ResolveError = errors::ResolveError;
    type DownloadError = errors::DownloadError;

    async fn refresh(&self, project: &Project) -> Result<(), Self::RefreshError> {
        match self {
            PackageSources::Pesde(source) => source.refresh(project).await.map_err(Into::into),
            #[cfg(feature = "wally-compat")]
            PackageSources::Wally(source) => source.refresh(project).await.map_err(Into::into),
            PackageSources::Git(source) => source.refresh(project).await.map_err(Into::into),
            PackageSources::Workspace(source) => source.refresh(project).await.map_err(Into::into),
        }
    }

    async fn resolve(
        &self,
        specifier: &Self::Specifier,
        project: &Project,
        project_target: TargetKind,
        refreshed_sources: &mut HashSet<PackageSources>,
    ) -> Result<ResolveResult<Self::Ref>, Self::ResolveError> {
        match (self, specifier) {
            (PackageSources::Pesde(source), DependencySpecifiers::Pesde(specifier)) => source
                .resolve(specifier, project, project_target, refreshed_sources)
                .await
                .map(|(name, results)| {
                    (
                        name,
                        results
                            .into_iter()
                            .map(|(version, pkg_ref)| (version, PackageRefs::Pesde(pkg_ref)))
                            .collect(),
                    )
                })
                .map_err(Into::into),

            #[cfg(feature = "wally-compat")]
            (PackageSources::Wally(source), DependencySpecifiers::Wally(specifier)) => source
                .resolve(specifier, project, project_target, refreshed_sources)
                .await
                .map(|(name, results)| {
                    (
                        name,
                        results
                            .into_iter()
                            .map(|(version, pkg_ref)| (version, PackageRefs::Wally(pkg_ref)))
                            .collect(),
                    )
                })
                .map_err(Into::into),

            (PackageSources::Git(source), DependencySpecifiers::Git(specifier)) => source
                .resolve(specifier, project, project_target, refreshed_sources)
                .await
                .map(|(name, results)| {
                    (
                        name,
                        results
                            .into_iter()
                            .map(|(version, pkg_ref)| (version, PackageRefs::Git(pkg_ref)))
                            .collect(),
                    )
                })
                .map_err(Into::into),

            (PackageSources::Workspace(source), DependencySpecifiers::Workspace(specifier)) => {
                source
                    .resolve(specifier, project, project_target, refreshed_sources)
                    .await
                    .map(|(name, results)| {
                        (
                            name,
                            results
                                .into_iter()
                                .map(|(version, pkg_ref)| {
                                    (version, PackageRefs::Workspace(pkg_ref))
                                })
                                .collect(),
                        )
                    })
                    .map_err(Into::into)
            }

            _ => Err(errors::ResolveError::Mismatch),
        }
    }

    async fn download(
        &self,
        pkg_ref: &Self::Ref,
        project: &Project,
        reqwest: &reqwest::Client,
    ) -> Result<(PackageFS, Target), Self::DownloadError> {
        match (self, pkg_ref) {
            (PackageSources::Pesde(source), PackageRefs::Pesde(pkg_ref)) => source
                .download(pkg_ref, project, reqwest)
                .await
                .map_err(Into::into),

            #[cfg(feature = "wally-compat")]
            (PackageSources::Wally(source), PackageRefs::Wally(pkg_ref)) => source
                .download(pkg_ref, project, reqwest)
                .await
                .map_err(Into::into),

            (PackageSources::Git(source), PackageRefs::Git(pkg_ref)) => source
                .download(pkg_ref, project, reqwest)
                .await
                .map_err(Into::into),

            (PackageSources::Workspace(source), PackageRefs::Workspace(pkg_ref)) => source
                .download(pkg_ref, project, reqwest)
                .await
                .map_err(Into::into),

            _ => Err(errors::DownloadError::Mismatch),
        }
    }
}

/// Errors that can occur when interacting with a package source
pub mod errors {
    use thiserror::Error;

    /// Errors that occur when refreshing a package source
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum RefreshError {
        /// A git-based package source failed to refresh
        #[error("error refreshing pesde package source")]
        GitBased(#[from] crate::source::git_index::errors::RefreshError),

        /// A workspace package source failed to refresh
        #[error("error refreshing workspace package source")]
        Workspace(#[from] crate::source::workspace::errors::RefreshError),
    }

    /// Errors that can occur when resolving a package
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum ResolveError {
        /// The dependency specifier does not match the source (if using the CLI, this is a bug - file an issue)
        #[error("mismatched dependency specifier for source")]
        Mismatch,

        /// A pesde package source failed to resolve
        #[error("error resolving pesde package")]
        Pesde(#[from] crate::source::pesde::errors::ResolveError),

        /// A Wally package source failed to resolve
        #[cfg(feature = "wally-compat")]
        #[error("error resolving wally package")]
        Wally(#[from] crate::source::wally::errors::ResolveError),

        /// A Git package source failed to resolve
        #[error("error resolving git package")]
        Git(#[from] crate::source::git::errors::ResolveError),

        /// A workspace package source failed to resolve
        #[error("error resolving workspace package")]
        Workspace(#[from] crate::source::workspace::errors::ResolveError),
    }

    /// Errors that can occur when downloading a package
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum DownloadError {
        /// The package ref does not match the source (if using the CLI, this is a bug - file an issue)
        #[error("mismatched package ref for source")]
        Mismatch,

        /// A pesde package source failed to download
        #[error("error downloading pesde package")]
        Pesde(#[from] crate::source::pesde::errors::DownloadError),

        /// A Wally package source failed to download
        #[cfg(feature = "wally-compat")]
        #[error("error downloading wally package")]
        Wally(#[from] crate::source::wally::errors::DownloadError),

        /// A Git package source failed to download
        #[error("error downloading git package")]
        Git(#[from] crate::source::git::errors::DownloadError),

        /// A workspace package source failed to download
        #[error("error downloading workspace package")]
        Workspace(#[from] crate::source::workspace::errors::DownloadError),
    }
}
