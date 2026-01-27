use std::path::PathBuf;

use crate::ser_display_deser_fromstr;
use crate::source::PackageRef;
use crate::source::StructureKind;
use crate::source::path::RelativeOrAbsolutePath;

/// A path package reference
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathPackageRef {
	/// The path of the package
	pub path: RelativeOrAbsolutePath,
	/// The absolute path to the package
	/// Only used internally and not user-visible
	pub absolute_path: PathBuf,
}
ser_display_deser_fromstr!(PathPackageRef);

impl PackageRef for PathPackageRef {
	fn structure_kind(&self) -> StructureKind {
		StructureKind::PesdeV1
	}
}

impl std::fmt::Display for PathPackageRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.path)
	}
}

/// Errors that can occur when parsing a path package reference
pub type PathPackageRefParseError = std::convert::Infallible;

impl std::str::FromStr for PathPackageRef {
	type Err = PathPackageRefParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(PathPackageRef {
			path: s.parse().unwrap(),
			absolute_path: PathBuf::new(),
		})
	}
}
