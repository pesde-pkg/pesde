//! Path package reference
use std::path::PathBuf;

use crate::ser_display_deser_fromstr;
use crate::source::PackageRef;

/// A path package reference
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathPackageRef {
	/// The path of the package
	pub path: PathBuf,
}
ser_display_deser_fromstr!(PathPackageRef);

impl PackageRef for PathPackageRef {}

impl std::fmt::Display for PathPackageRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.path.display())
	}
}

/// Errors that can occur when parsing a path package reference
#[derive(Debug, thiserror::Error)]
pub enum PathPackageRefParseError {}

impl std::str::FromStr for PathPackageRef {
	type Err = PathPackageRefParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(PathPackageRef {
			path: s.parse().unwrap(),
		})
	}
}
