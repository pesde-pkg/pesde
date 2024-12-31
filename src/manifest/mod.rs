use crate::{
    manifest::{
        overrides::{OverrideKey, OverrideSpecifier},
        target::Target,
    },
    names::PackageName,
    source::specifiers::DependencySpecifiers,
};
use relative_path::RelativePathBuf;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use tracing::instrument;

/// Overrides
pub mod overrides;
/// Targets
pub mod target;

/// A package manifest
#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Manifest {
    /// The name of the package
    pub name: PackageName,
    /// The version of the package
    pub version: Version,
    /// The description of the package
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The license of the package
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// The authors of the package
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// The repository of the package
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<url::Url>,
    /// The target of the package
    pub target: Target,
    /// Whether the package is private
    #[serde(default)]
    pub private: bool,
    /// The scripts of the package
    #[serde(default, skip_serializing)]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "BTreeMap<String, std::path::PathBuf>")
    )]
    pub scripts: BTreeMap<String, RelativePathBuf>,
    /// The indices to use for the package
    #[serde(
        default,
        skip_serializing,
        deserialize_with = "crate::util::deserialize_gix_url_map"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "BTreeMap<String, url::Url>"))]
    pub indices: BTreeMap<String, gix::Url>,
    /// The indices to use for the package's wally dependencies
    #[cfg(feature = "wally-compat")]
    #[serde(
        default,
        skip_serializing,
        deserialize_with = "crate::util::deserialize_gix_url_map"
    )]
    #[cfg_attr(feature = "schema", schemars(with = "BTreeMap<String, url::Url>"))]
    pub wally_indices: BTreeMap<String, gix::Url>,
    /// The overrides this package has
    #[serde(default, skip_serializing)]
    pub overrides: BTreeMap<OverrideKey, OverrideSpecifier>,
    /// The files to include in the package
    #[serde(default)]
    pub includes: Vec<String>,
    /// The patches to apply to packages
    #[cfg(feature = "patches")]
    #[serde(default, skip_serializing)]
    #[cfg_attr(
        feature = "schema",
        schemars(
            with = "BTreeMap<crate::names::PackageNames, BTreeMap<crate::source::version_id::VersionId, std::path::PathBuf>>"
        )
    )]
    pub patches: BTreeMap<
        crate::names::PackageNames,
        BTreeMap<crate::source::version_id::VersionId, RelativePathBuf>,
    >,
    #[serde(default, skip_serializing)]
    /// Which version of the pesde CLI this package uses
    pub pesde_version: Option<Version>,
    /// A list of globs pointing to workspace members' directories
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_members: Vec<String>,
    /// The Roblox place of this project
    #[serde(default, skip_serializing)]
    pub place: BTreeMap<target::RobloxPlaceKind, String>,

    /// The standard dependencies of the package
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, DependencySpecifiers>,
    /// The peer dependencies of the package
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub peer_dependencies: BTreeMap<String, DependencySpecifiers>,
    /// The dev dependencies of the package
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dev_dependencies: BTreeMap<String, DependencySpecifiers>,
    /// The user-defined fields of the package
    #[cfg_attr(feature = "schema", schemars(skip))]
    #[serde(flatten)]
    pub user_defined_fields: HashMap<String, toml::Value>,
}

/// A dependency type
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DependencyType {
    /// A standard dependency
    Standard,
    /// A peer dependency
    Peer,
    /// A dev dependency
    Dev,
}

impl Manifest {
    /// Get all dependencies from the manifest
    #[instrument(skip(self), ret(level = "trace"), level = "debug")]
    pub fn all_dependencies(
        &self,
    ) -> Result<
        BTreeMap<String, (DependencySpecifiers, DependencyType)>,
        errors::AllDependenciesError,
    > {
        let mut all_deps = BTreeMap::new();

        for (deps, ty) in [
            (&self.dependencies, DependencyType::Standard),
            (&self.peer_dependencies, DependencyType::Peer),
            (&self.dev_dependencies, DependencyType::Dev),
        ] {
            for (alias, spec) in deps {
                if all_deps.insert(alias.clone(), (spec.clone(), ty)).is_some() {
                    return Err(errors::AllDependenciesError::AliasConflict(alias.clone()));
                }
            }
        }

        Ok(all_deps)
    }
}

/// Errors that can occur when interacting with manifests
pub mod errors {
    use thiserror::Error;

    /// Errors that can occur when trying to get all dependencies from a manifest
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum AllDependenciesError {
        /// Another specifier is already using the alias
        #[error("another specifier is already using the alias {0}")]
        AliasConflict(String),
    }
}
