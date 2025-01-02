use std::fmt::Display;

use semver::VersionReq;
use serde::{Deserialize, Serialize};

use crate::{names::wally::WallyPackageName, source::DependencySpecifier};

/// The specifier for a Wally dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct WallyDependencySpecifier {
	/// The name of the package
	#[serde(rename = "wally")]
	pub name: WallyPackageName,
	/// The version requirement for the package
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub version: VersionReq,
	/// The index to use for the package
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub index: Option<String>,
}
impl DependencySpecifier for WallyDependencySpecifier {}

impl Display for WallyDependencySpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}@{}", self.name, self.version)
	}
}
