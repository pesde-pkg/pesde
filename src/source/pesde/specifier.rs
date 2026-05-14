//! pesde dependency specifier
use crate::names::PackageName;
use crate::source::DependencySpecifier;
use crate::source::Realm;
use semver::VersionReq;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashSet;
use std::fmt::Display;

pub(crate) fn matches(keys: &HashSet<&str>) -> bool {
	keys.contains(&"name") && !keys.contains(&"target")
}

/// The specifier for a pesde dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct PesdeDependencySpecifier {
	/// The name of the package
	pub name: PackageName,
	/// The version requirement for the package
	pub version: VersionReq,
	/// The registry to use for the package
	#[serde(default = "crate::default_url_key")]
	pub registry: String,
	/// The realm to use for the package
	pub realm: Option<Realm>,
}
impl DependencySpecifier for PesdeDependencySpecifier {
	fn realm(&self) -> Option<Realm> {
		self.realm
	}
}

impl Display for PesdeDependencySpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}@{}", self.name, self.version)
	}
}

/// The specifier of a pesde dependency from a pesde registry
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct RegistryPesdeDependencySpecifier {
	/// The name of the package
	pub name: PackageName,
	/// The version requirement for the package
	pub version: VersionReq,
	/// The registry to use for the package. None if this package comes from the same registry
	pub registry: Option<url::Url>,
	/// The realm to use for the package
	pub realm: Option<Realm>,
}
