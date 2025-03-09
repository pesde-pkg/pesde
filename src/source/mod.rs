use crate::{
	manifest::target::{Target, TargetKind},
	names::PackageNames,
	reporters::DownloadProgressReporter,
	source::{
		fs::PackageFs, ids::VersionId, refs::PackageRefs, specifiers::DependencySpecifiers,
		traits::*,
	},
};
use std::{
	collections::{BTreeMap, BTreeSet},
	fmt::Debug,
};

/// Packages' filesystems
pub mod fs;
/// The Git package source
pub mod git;
/// Git index-based package source utilities
pub mod git_index;
/// Package identifiers for different contexts
pub mod ids;
/// The path package source
pub mod path;
/// The pesde package source
pub mod pesde;
/// Package references
pub mod refs;
/// Dependency specifiers
pub mod specifiers;
/// Traits for sources and packages
pub mod traits;
/// The Wally package source
#[cfg(feature = "wally-compat")]
pub mod wally;
/// The workspace package source
pub mod workspace;

/// Files that will not be stored when downloading a package. These are only files which break pesde's functionality, or are meaningless and possibly heavy (e.g. `.DS_Store`)
pub const IGNORED_FILES: &[&str] = &["foreman.toml", "aftman.toml", "rokit.toml", ".DS_Store"];

/// Files that should be ignored in some contexts, usually only pesde packages
pub const ADDITIONAL_FORBIDDEN_FILES: &[&str] = &["default.project.json"];

/// Directories that will not be stored when downloading a package. These are only directories which break pesde's functionality, or are meaningless and possibly heavy
pub const IGNORED_DIRS: &[&str] = &[".git"];

/// The result of resolving a package
pub type ResolveResult<Ref> = (PackageNames, BTreeMap<VersionId, Ref>, BTreeSet<TargetKind>);

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
	/// A path package source
	Path(path::PathPackageSource),
}

impl PackageSource for PackageSources {
	type Specifier = DependencySpecifiers;
	type Ref = PackageRefs;
	type RefreshError = errors::RefreshError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetTargetError = errors::GetTargetError;

	async fn refresh(&self, options: &RefreshOptions) -> Result<(), Self::RefreshError> {
		match self {
			PackageSources::Pesde(source) => source
				.refresh(options)
				.await
				.map_err(Self::RefreshError::Pesde),
			#[cfg(feature = "wally-compat")]
			PackageSources::Wally(source) => source
				.refresh(options)
				.await
				.map_err(Self::RefreshError::Wally),
			PackageSources::Git(source) => source
				.refresh(options)
				.await
				.map_err(Self::RefreshError::Git),
			PackageSources::Workspace(source) => source.refresh(options).await.map_err(Into::into),
			PackageSources::Path(source) => source.refresh(options).await.map_err(Into::into),
		}
	}

	async fn resolve(
		&self,
		specifier: &Self::Specifier,
		options: &ResolveOptions,
	) -> Result<ResolveResult<Self::Ref>, Self::ResolveError> {
		match (self, specifier) {
			(PackageSources::Pesde(source), DependencySpecifiers::Pesde(specifier)) => source
				.resolve(specifier, options)
				.await
				.map(|(name, results, suggestions)| {
					(
						name,
						results
							.into_iter()
							.map(|(version, pkg_ref)| (version, PackageRefs::Pesde(pkg_ref)))
							.collect(),
						suggestions,
					)
				})
				.map_err(Into::into),

			#[cfg(feature = "wally-compat")]
			(PackageSources::Wally(source), DependencySpecifiers::Wally(specifier)) => source
				.resolve(specifier, options)
				.await
				.map(|(name, results, suggestions)| {
					(
						name,
						results
							.into_iter()
							.map(|(version, pkg_ref)| (version, PackageRefs::Wally(pkg_ref)))
							.collect(),
						suggestions,
					)
				})
				.map_err(Into::into),

			(PackageSources::Git(source), DependencySpecifiers::Git(specifier)) => source
				.resolve(specifier, options)
				.await
				.map(|(name, results, suggestions)| {
					(
						name,
						results
							.into_iter()
							.map(|(version, pkg_ref)| (version, PackageRefs::Git(pkg_ref)))
							.collect(),
						suggestions,
					)
				})
				.map_err(Into::into),

			(PackageSources::Workspace(source), DependencySpecifiers::Workspace(specifier)) => {
				source
					.resolve(specifier, options)
					.await
					.map(|(name, results, suggestions)| {
						(
							name,
							results
								.into_iter()
								.map(|(version, pkg_ref)| {
									(version, PackageRefs::Workspace(pkg_ref))
								})
								.collect(),
							suggestions,
						)
					})
					.map_err(Into::into)
			}

			(PackageSources::Path(source), DependencySpecifiers::Path(specifier)) => source
				.resolve(specifier, options)
				.await
				.map(|(name, results, suggestions)| {
					(
						name,
						results
							.into_iter()
							.map(|(version, pkg_ref)| (version, PackageRefs::Path(pkg_ref)))
							.collect(),
						suggestions,
					)
				})
				.map_err(Into::into),

			_ => Err(errors::ResolveError::Mismatch),
		}
	}

	async fn download<R: DownloadProgressReporter>(
		&self,
		pkg_ref: &Self::Ref,
		options: &DownloadOptions<R>,
	) -> Result<PackageFs, Self::DownloadError> {
		match (self, pkg_ref) {
			(PackageSources::Pesde(source), PackageRefs::Pesde(pkg_ref)) => {
				source.download(pkg_ref, options).await.map_err(Into::into)
			}

			#[cfg(feature = "wally-compat")]
			(PackageSources::Wally(source), PackageRefs::Wally(pkg_ref)) => {
				source.download(pkg_ref, options).await.map_err(Into::into)
			}

			(PackageSources::Git(source), PackageRefs::Git(pkg_ref)) => {
				source.download(pkg_ref, options).await.map_err(Into::into)
			}

			(PackageSources::Workspace(source), PackageRefs::Workspace(pkg_ref)) => {
				source.download(pkg_ref, options).await.map_err(Into::into)
			}

			(PackageSources::Path(source), PackageRefs::Path(pkg_ref)) => {
				source.download(pkg_ref, options).await.map_err(Into::into)
			}

			_ => Err(errors::DownloadError::Mismatch),
		}
	}

	async fn get_target(
		&self,
		pkg_ref: &Self::Ref,
		options: &GetTargetOptions,
	) -> Result<Target, Self::GetTargetError> {
		match (self, pkg_ref) {
			(PackageSources::Pesde(source), PackageRefs::Pesde(pkg_ref)) => source
				.get_target(pkg_ref, options)
				.await
				.map_err(Into::into),

			#[cfg(feature = "wally-compat")]
			(PackageSources::Wally(source), PackageRefs::Wally(pkg_ref)) => source
				.get_target(pkg_ref, options)
				.await
				.map_err(Into::into),

			(PackageSources::Git(source), PackageRefs::Git(pkg_ref)) => source
				.get_target(pkg_ref, options)
				.await
				.map_err(Into::into),

			(PackageSources::Workspace(source), PackageRefs::Workspace(pkg_ref)) => source
				.get_target(pkg_ref, options)
				.await
				.map_err(Into::into),

			(PackageSources::Path(source), PackageRefs::Path(pkg_ref)) => source
				.get_target(pkg_ref, options)
				.await
				.map_err(Into::into),

			_ => Err(errors::GetTargetError::Mismatch),
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
		/// A pesde package source failed to refresh
		#[error("error refreshing pesde package source")]
		Pesde(#[source] crate::source::git_index::errors::RefreshError),

		/// A Wally package source failed to refresh
		#[cfg(feature = "wally-compat")]
		#[error("error refreshing wally package source")]
		Wally(#[source] crate::source::git_index::errors::RefreshError),

		/// A Git package source failed to refresh
		#[error("error refreshing git package source")]
		Git(#[source] crate::source::git_index::errors::RefreshError),

		/// A workspace package source failed to refresh
		#[error("error refreshing workspace package source")]
		Workspace(#[from] crate::source::workspace::errors::RefreshError),

		/// A path package source failed to refresh
		#[error("error refreshing path package source")]
		Path(#[from] crate::source::path::errors::RefreshError),
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

		/// A path package source failed to resolve
		#[error("error resolving path package")]
		Path(#[from] crate::source::path::errors::ResolveError),
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

		/// A path package source failed to download
		#[error("error downloading path package")]
		Path(#[from] crate::source::path::errors::DownloadError),
	}

	/// Errors that can occur when getting a package's target
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum GetTargetError {
		/// The package ref does not match the source (if using the CLI, this is a bug - file an issue)
		#[error("mismatched package ref for source")]
		Mismatch,

		/// A pesde package source failed to get the target
		#[error("error getting target for pesde package")]
		Pesde(#[from] crate::source::pesde::errors::GetTargetError),

		/// A Wally package source failed to get the target
		#[cfg(feature = "wally-compat")]
		#[error("error getting target for wally package")]
		Wally(#[from] crate::source::wally::errors::GetTargetError),

		/// A Git package source failed to get the target
		#[error("error getting target for git package")]
		Git(#[from] crate::source::git::errors::GetTargetError),

		/// A workspace package source failed to get the target
		#[error("error getting target for workspace package")]
		Workspace(#[from] crate::source::workspace::errors::GetTargetError),

		/// A path package source failed to get the target
		#[error("error getting target for path package")]
		Path(#[from] crate::source::path::errors::GetTargetError),
	}
}
