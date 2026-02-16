use relative_path::RelativePathBuf;
use semver::Version;

use crate::Project;
use crate::RefreshedSources;
use crate::Subproject;
use crate::reporters::DownloadProgressReporter;
use crate::resolver::DependencyGraph;
use crate::source::PackageFs;
use crate::source::Realm;
use crate::source::ResolveResult;
use crate::source::StructureKind;
use std::fmt::Debug;
use std::fmt::Display;
use std::future;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

/// A specifier for a dependency
pub trait DependencySpecifier: Debug + Display {
	/// The realm this dependency is for, if any
	fn realm(&self) -> Option<Realm>;
}

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
	/// The sources that have been refreshed
	pub refreshed_sources: RefreshedSources,
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
	/// The version of the package to be downloaded
	pub version: &'a Version,
}

/// Options for getting a package's Target
#[derive(Debug, Clone)]
pub struct GetExportsOptions<'a> {
	/// The project to get the target for
	pub project: Project,
	/// The path the package has been written to
	pub path: Arc<Path>,
	/// The version of the package to be downloaded
	pub version: &'a Version,
}

/// The exports of a package
#[derive(Debug, Clone)]
pub struct PackageExports {
	/// The path to the lib export file
	pub lib_file: Option<RelativePathBuf>,
	/// The path to the bin export file
	pub bin_file: Option<RelativePathBuf>,
	/// The x script export of this package, if any
	pub x_script: Option<String>,
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
	/// The error type for getting a package's exports from this source
	type GetExportsError: std::error::Error + Send + Sync + 'static;

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

	/// Gets the exports of a package
	fn get_exports(
		&self,
		pkg_ref: &Self::Ref,
		options: &GetExportsOptions<'_>,
	) -> impl Future<Output = Result<PackageExports, Self::GetExportsError>> + Send;
}
