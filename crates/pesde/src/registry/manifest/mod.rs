use serde::Deserialize;
use serde::Serialize;

use crate::bounded::BoundedBTreeMap;
use crate::bounded::BoundedStr;
use crate::bounded::BoundedUrl;
use crate::registry::PackageName;
use crate::registry::PesdeStyleVersion;

mod alias;
pub use alias::*;
mod exports;
pub use exports::*;
mod dependencies;
pub use dependencies::*;
pub mod config_luau;
pub mod wally_compat;

pub const MAX_DESCRIPTION_LEN: usize = 255;
pub const MAX_LICENSE_LEN: usize = 48;
pub const MAX_URL_LEN: usize = 2048;
pub const MAX_DEPENDENCIES: usize = 65535;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestPackage {
	/// The name of the package
	pub name: PackageName,
	/// The version of the package
	pub version: PesdeStyleVersion,
	/// The description of the package
	#[serde(default, skip_serializing_if = "str::is_empty")]
	pub description: BoundedStr<MAX_DESCRIPTION_LEN>,
	/// The license of the package
	#[serde(default, skip_serializing_if = "str::is_empty")]
	pub license: BoundedStr<MAX_LICENSE_LEN>,
	/// The repository of the package
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub repository: Option<BoundedUrl<MAX_URL_LEN>>,
}

/// A resolved manifest, that is **NOT** the user-facing manifest.
/// This type is the result of PARSING that, and is optimised for working with the data,
/// e.g. it contains one dependencies map, rather than multiple maps for each dependency type.
/// It also doesn't store information that is not relevant to the registry, e.g. the workspace field or
/// patches.
/// This is what travels in published packages, and is what is used in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedManifest {
	/// The package information
	pub package: ManifestPackage,
	/// The exports of the package
	pub exports: ManifestExports,
	/// The Luau configuration of the package
	pub config_luau: config_luau::LuauConfig,
	/// The dependencies of the package
	pub dependencies:
		BoundedBTreeMap<Alias, (ResolvedDependencySpecifier, DependencyType), MAX_DEPENDENCIES>,
}
