use serde::{Deserialize, Serialize};

use crate::{
	bounded::{BoundedBTreeMap, BoundedSlice, BoundedStr, BoundedUrl},
	registry::{PackageName, PesdeStyleVersion},
};

mod alias;
pub use alias::*;
mod exports;
pub use exports::*;
mod dependencies;
pub use dependencies::*;
pub mod wally_compat;

pub const MAX_DESCRIPTION_LEN: usize = 255;
pub const MAX_AUTHORS: usize = 25;
pub const MAX_AUTHOR_LEN: usize = 255;
pub const MAX_EMAIL_LEN: usize = 255;
pub const MAX_URL_LEN: usize = 2048;
pub const MAX_SCRIPTS: usize = 255;
pub const MAX_SCRIPT_NAME_LEN: usize = 255;
pub const MAX_SCRIPT_LEN: usize = 2048;
pub const MAX_DEPENDENCIES: usize = 255;

fn true_bool() -> bool {
	true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestPackage {
	/// The name of the package
	pub name: PackageName,
	/// The version of the package
	pub version: PesdeStyleVersion,
	/// Whether the package should be published
	#[serde(default = "true_bool", skip_serializing_if = "Clone::clone")]
	pub publish: bool,
	/// The description of the package
	#[serde(default, skip_serializing_if = "str::is_empty")]
	pub description: BoundedStr<MAX_DESCRIPTION_LEN>,
	/// The license of the package
	#[serde(default, skip_serializing_if = "str::is_empty")]
	pub license: BoundedStr<MAX_DESCRIPTION_LEN>,
	/// The authors of the package
	#[serde(default, skip_serializing_if = "BoundedSlice::is_empty")]
	pub authors: BoundedSlice<BoundedStr<MAX_AUTHOR_LEN>, MAX_AUTHORS>,
	/// The repository of the package
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub repository: Option<BoundedUrl<MAX_URL_LEN>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAuthor {
	/// The name of the author
	pub name: BoundedStr<MAX_AUTHOR_LEN>,
	/// The email of the author
	#[serde(default, skip_serializing_if = "str::is_empty")]
	pub email: BoundedStr<MAX_EMAIL_LEN>,
	/// The URL of the author
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub url: Option<BoundedUrl<MAX_URL_LEN>>,
}

/// A resolved manifest, that is **NOT** the user-facing manifest.
/// This type is the result of PARSING that, and is optimised for working with the data,
/// e.g. it contains one dependencies map, rather than multiple maps for each dependency type.
/// It also doesn't store information that is not relevant to the registry, e.g. the workspace field or
/// patches.
/// This is what travels in published packages, and is what is used in the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedManifest {
	/// The package information
	pub package: ManifestPackage,
	/// The exports of the package
	pub exports: ManifestExports,
	/// The scripts of the package
	pub scripts:
		BoundedBTreeMap<BoundedStr<MAX_SCRIPT_NAME_LEN>, BoundedStr<MAX_SCRIPT_LEN>, MAX_SCRIPTS>,
	/// The dependencies of the package
	pub dependencies:
		BoundedBTreeMap<Alias, (ResolvedDependencySpecifier, DependencyType), MAX_DEPENDENCIES>,
}
