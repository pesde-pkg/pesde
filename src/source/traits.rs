use crate::Project;
use crate::RefreshedSources;
use crate::Subproject;
use crate::engine::runtime::Engines;
use crate::manifest::target::Target;
use crate::manifest::target::TargetKind;
use crate::reporters::DownloadProgressReporter;
use crate::source::PackageFs;
use crate::source::ResolveResult;
use crate::source::ids::VersionId;
use crate::source::refs::StructureKind;
use std::fmt::Debug;
use std::fmt::Display;
use std::future::Future;
use std::future::{self};
use std::path::Path;
use std::sync::Arc;

/// A specifier for a dependency
pub trait DependencySpecifier: Debug + Display {}

/// A reference to a package
pub trait PackageRef: Debug {
	/// The kind of structure this package uses
	fn structure_kind(&self) -> StructureKind;
}

/// Options for refreshing a source
#[derive(Debug, Clone)]
pub struct RefreshOptions {
	/// The project to refresh for
	pub project: Project,
}

/// Options for resolving a package
#[derive(Debug, Clone)]
pub struct ResolveOptions {
	/// The subproject to resolve for
	pub subproject: Subproject,
	/// The target to resolve for
	pub target: TargetKind,
	/// The sources that have been refreshed
	pub refreshed_sources: RefreshedSources,
	/// Whether to find any compatible target instead of a strict equal. Each source defines its
	/// own loose rules.
	pub loose_target: bool,
}

/// Options for downloading a package
#[derive(Debug, Clone)]
pub struct DownloadOptions<'a, R: DownloadProgressReporter> {
	/// The project to download for
	pub project: Project,
	/// The reqwest client to use
	pub reqwest: reqwest::Client,
	/// The reporter to use
	pub reporter: Arc<R>,
	/// The version ID of the package to be downloaded
	pub version_id: &'a VersionId,
}

/// Options for getting a package's Target
#[derive(Debug, Clone)]
pub struct GetTargetOptions<'a> {
	/// The project to get the target for
	pub project: Project,
	/// The path the package has been written to
	pub path: Arc<Path>,
	/// The version ID of the package to be downloaded
	pub version_id: &'a VersionId,
	/// The engines this project is using
	pub engines: Arc<Engines>,
}

/// A source of packages
pub trait PackageSource: Debug {
	/// The specifier type for this source
	type Specifier: DependencySpecifier;
	/// The reference type for this source
	type Ref: PackageRef;
	/// The error type for refreshing this source
	type RefreshError: std::error::Error + Send + Sync + 'static;
	/// The error type for resolving a package from this source
	type ResolveError: std::error::Error + Send + Sync + 'static;
	/// The error type for downloading a package from this source
	type DownloadError: std::error::Error + Send + Sync + 'static;
	/// The error type for getting a package's target from this source
	type GetTargetError: std::error::Error + Send + Sync + 'static;

	/// Refreshes the source
	fn refresh(
		&self,
		_options: &RefreshOptions,
	) -> impl Future<Output = Result<(), Self::RefreshError>> + Send {
		future::ready(Ok(()))
	}

	/// Resolves a specifier to a reference
	fn resolve(
		&self,
		specifier: &Self::Specifier,
		options: &ResolveOptions,
	) -> impl Future<Output = Result<ResolveResult, Self::ResolveError>> + Send;

	/// Downloads a package
	fn download<R: DownloadProgressReporter>(
		&self,
		pkg_ref: &Self::Ref,
		options: &DownloadOptions<'_, R>,
	) -> impl Future<Output = Result<PackageFs, Self::DownloadError>> + Send;

	/// Gets the target of a package
	fn get_target(
		&self,
		pkg_ref: &Self::Ref,
		options: &GetTargetOptions<'_>,
	) -> impl Future<Output = Result<Target, Self::GetTargetError>> + Send;
}
