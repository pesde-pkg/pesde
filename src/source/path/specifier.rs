//! Path dependency specifier
use crate::source::DependencySpecifier;
use crate::source::Realm;
use crate::source::path::RelativeOrAbsolutePath;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashSet;
use std::fmt::Display;

pub(crate) fn matches(keys: &HashSet<&str>) -> bool {
	keys.contains(&"path")
}

/// The specifier for a path dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct PathDependencySpecifier {
	/// The path to the package
	pub path: RelativeOrAbsolutePath,
	/// The realm of the package
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub realm: Option<Realm>,
}
impl DependencySpecifier for PathDependencySpecifier {
	fn realm(&self) -> Option<Realm> {
		self.realm
	}
}

impl Display for PathDependencySpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "path:{}", self.path)
	}
}
