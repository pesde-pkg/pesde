//! Wally dependency specifier
use std::collections::HashSet;
use std::fmt::Display;

use semver::VersionReq;
use serde::Deserialize;
use serde::Serialize;

use crate::Url;
use crate::bounded::Bounded;
use crate::manifest::MAX_URL_LEN;
use crate::manifest::MAX_VERSION_REQ_LEN;
use crate::names::WallyPackageName;
use crate::source::DependencySpecifier;
use crate::source::Realm;

pub(crate) fn matches(keys: &HashSet<&str>) -> bool {
	keys.contains(&"wally")
}

/// The specifier for a Wally dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct WallyDependencySpecifier {
	/// The name of the package
	#[serde(rename = "wally")]
	pub name: WallyPackageName,
	/// The version requirement for the package
	pub version: VersionReq,
	/// The index to use for the package
	#[serde(default = "crate::default_url_key")]
	pub index: String,
	/// The realm to use for the package
	pub realm: Realm,
}
impl DependencySpecifier for WallyDependencySpecifier {
	fn realm(&self) -> Option<Realm> {
		// Wally packages aren't designed for standard Luau, only Roblox, so we should require a realm for them
		Some(self.realm)
	}
}

impl Display for WallyDependencySpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}@{}", self.name, self.version)
	}
}

/// The specifier for a Wally dependency in the index
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct IndexWallyDependencySpecifier {
	/// The name of the package
	#[serde(rename = "wally")]
	pub name: WallyPackageName,
	/// The version requirement for the package
	pub version: VersionReq,
	/// The index to use for the package
	pub index: String,
}

/// The specifier for a Wally dependency from a pesde registry
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct RegistryWallyDependencySpecifier {
	/// The name of the package
	pub name: WallyPackageName,
	/// The version requirement for the package
	pub version: Bounded<VersionReq, MAX_VERSION_REQ_LEN>,
	/// The index to use for the package
	pub index: Bounded<Url, MAX_URL_LEN>,
	/// The realm to use for the package
	pub realm: Realm,
}
