use super::target::TargetKind;
use crate::names::PackageName;
use crate::source::DependencySpecifier;
use crate::source::Realm;
use semver::VersionReq;
use serde::Deserialize;
use serde::Serialize;
use std::fmt::Display;

/// The specifier for a pesde dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct PesdeDependencySpecifier {
	/// The name of the package
	pub name: PackageName,
	/// The version requirement for the package
	pub version: VersionReq,
	/// The index to use for the package
	#[serde(default = "crate::default_index_name")]
	pub index: String,
	/// The target to use for the package
	pub target: TargetKind,
}
impl DependencySpecifier for PesdeDependencySpecifier {
	fn realm(&self) -> Option<Realm> {
		match self.target {
			TargetKind::Roblox => Some(Realm::Shared),
			TargetKind::RobloxServer => Some(Realm::Server),
			TargetKind::Lune | TargetKind::Luau => None,
		}
	}
}

impl Display for PesdeDependencySpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}@{} {}", self.name, self.version, self.target)
	}
}
