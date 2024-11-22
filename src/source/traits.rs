#![allow(async_fn_in_trait)]
use crate::{
    manifest::{
        target::{Target, TargetKind},
        DependencyType,
    },
    source::{DependencySpecifiers, PackageFS, PackageSources, ResolveResult},
    Project,
};
use std::{
    collections::{BTreeMap, HashSet},
    fmt::{Debug, Display},
};

/// A specifier for a dependency
pub trait DependencySpecifier: Debug + Display {}

/// A reference to a package
pub trait PackageRef: Debug {
    /// The dependencies of this package
    fn dependencies(&self) -> &BTreeMap<String, (DependencySpecifiers, DependencyType)>;
    /// Whether to use the new structure (`packages` folders inside the package's content folder) or the old structure (Wally-style, with linker files in the parent of the folder containing the package's contents)
    fn use_new_structure(&self) -> bool;
    /// The source of this package
    fn source(&self) -> PackageSources;
}

/// A source of packages
pub trait PackageSource: Debug {
    /// The specifier type for this source
    type Specifier: DependencySpecifier;
    /// The reference type for this source
    type Ref: PackageRef;
    /// The error type for refreshing this source
    type RefreshError: std::error::Error;
    /// The error type for resolving a package from this source
    type ResolveError: std::error::Error;
    /// The error type for downloading a package from this source
    type DownloadError: std::error::Error;

    /// Refreshes the source
    async fn refresh(&self, _project: &Project) -> Result<(), Self::RefreshError> {
        Ok(())
    }

    /// Resolves a specifier to a reference
    async fn resolve(
        &self,
        specifier: &Self::Specifier,
        project: &Project,
        project_target: TargetKind,
        refreshed_sources: &mut HashSet<PackageSources>,
    ) -> Result<ResolveResult<Self::Ref>, Self::ResolveError>;

    /// Downloads a package
    async fn download(
        &self,
        pkg_ref: &Self::Ref,
        project: &Project,
        reqwest: &reqwest::Client,
    ) -> Result<(PackageFS, Target), Self::DownloadError>;
}
