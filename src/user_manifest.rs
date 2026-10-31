//! Manifest
use crate::Url;
use crate::manifest::*;
use crate::source::DependencySpecifiers;
use crate::source::PackageExports;
use crate::source::Realm;
use crate::source::ids::PackageId;
use relative_path::RelativePathBuf;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;
use tracing::instrument;

/// A specifier for an override
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum OverrideSpecifier {
	/// A specifier for a dependency
	Specifier(DependencySpecifiers),
	/// An alias for a dependency the current project depends on
	Alias(Alias),
}

/// The `workspace` field of the manifest
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ManifestWorkspace {
	/// A list of globs pointing to workspace members' directories
	pub members: Vec<String>,
	/// The patches to apply to packages
	#[cfg(feature = "patches")]
	pub patches: BTreeMap<PackageId, RelativePathBuf>,
	/// The overrides this workspace has
	pub overrides: BTreeMap<PackageId, OverrideSpecifier>,
}

/// A package manifest
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
	/// Whether the package is private
	#[serde(default)]
	pub private: bool,
	/// The registries to use for the package
	#[serde(default, rename = "registries")]
	pub pesde_registries: BTreeMap<String, Bounded<Url, MAX_URL_LEN>>,
	/// The indices to use for the package
	#[serde(default, rename = "indices")]
	pub pesde_indices: BTreeMap<String, Bounded<Url, MAX_URL_LEN>>,
	/// The indices to use for the package's Wally dependencies
	#[serde(default, rename = "wally_indices")]
	pub wally_indices: BTreeMap<String, Bounded<Url, MAX_URL_LEN>>,
	/// The files to include in the package
	#[serde(default)]
	pub includes: Vec<String>,
	/// The workspace configuration
	#[serde(default)]
	pub workspace: ManifestWorkspace,
	/// The absolute paths of package directories in this project
	#[serde(default)]
	pub absolute_paths: BTreeMap<Realm, String>,
	/// The lib export of this package
	#[serde(default)]
	pub lib: Option<RelativePathBuf>,
	/// The bin export of this package
	#[serde(default)]
	pub bin: Option<RelativePathBuf>,

	/// The standard dependencies of the package
	#[serde(default, deserialize_with = "crate::util::deserialize_no_dup_keys")]
	pub dependencies: BTreeMap<Alias, DependencySpecifiers>,
	/// The peer dependencies of the package
	#[serde(default, deserialize_with = "crate::util::deserialize_no_dup_keys")]
	pub peer_dependencies: BTreeMap<Alias, DependencySpecifiers>,
	/// The dev dependencies of the package
	#[serde(default, deserialize_with = "crate::util::deserialize_no_dup_keys")]
	pub dev_dependencies: BTreeMap<Alias, DependencySpecifiers>,

	/// An area for user-defined fields, which will always be ignored by pesde
	#[serde(default)]
	pub meta: HashMap<String, toml::Value>,
}

impl Manifest {
	/// Get all dependencies from the manifest
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub fn all_dependencies(
		&self,
	) -> Result<BTreeMap<Alias, (DependencySpecifiers, DependencyType)>, errors::AllDependenciesError>
	{
		let mut all_deps = BTreeMap::new();

		for (deps, ty) in [
			(&self.dependencies, DependencyType::Standard),
			(&self.peer_dependencies, DependencyType::Peer),
			(&self.dev_dependencies, DependencyType::Dev),
		] {
			for (alias, spec) in deps {
				if all_deps.insert(alias.clone(), (spec.clone(), ty)).is_some() {
					return Err(
						errors::AllDependenciesErrorKind::AliasConflict(alias.clone()).into(),
					);
				}
			}
		}

		Ok(all_deps)
	}

	/// Converts the manifest into a [PackageExports]
	#[must_use]
	pub fn as_exports(&self) -> PackageExports {
		PackageExports {
			lib_file: self.lib.clone(),
			bin_file: self.bin.clone(),
		}
	}
}

/// Errors that can occur when interacting with manifests
pub mod errors {
	use crate::user_manifest::Alias;
	use thiserror::Error;

	/// Errors that can occur when trying to get all dependencies from a manifest
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = AllDependenciesError))]
	#[non_exhaustive]
	pub enum AllDependenciesErrorKind {
		/// Another specifier is already using the alias
		#[error("another specifier is already using the alias {0}")]
		AliasConflict(Alias),
	}
}
