use crate::{manifest::target::TargetKind, names::PackageName, source::DependencySpecifier};
use semver::VersionReq;
use serde::{Deserialize, Serialize};
use std::fmt::Display;

/// The specifier for a pesde dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PesdeDependencySpecifier {
	/// The name of the package
	pub name: PackageName,
	/// The version requirement for the package
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub version: VersionReq,
	/// The index to use for the package
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub index: Option<String>,
	/// The target to use for the package
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub target: Option<TargetKind>,
}
impl DependencySpecifier for PesdeDependencySpecifier {}

impl Display for PesdeDependencySpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}@{}", self.name, self.version)
	}
}
