use std::fmt::Display;

use semver::VersionReq;
use serde::Deserialize;
use serde::Serialize;

use crate::names::wally::WallyPackageName;
use crate::source::DependencySpecifier;
use crate::source::Realm;

/// The specifier for a Wally dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct WallyDependencySpecifier {
	/// The name of the package
	#[serde(rename = "wally")]
	pub name: WallyPackageName,
	/// The version requirement for the package
	pub version: VersionReq,
	/// The index to use for the package
	#[serde(default = "crate::default_index_name")]
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
