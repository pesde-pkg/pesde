use crate::{
	ser_display_deser_fromstr,
	source::{PackageRef, refs::StructureKind},
};
use std::path::PathBuf;

/// A path package reference
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathPackageRef {
	/// The path of the package
	pub path: PathBuf,
}
ser_display_deser_fromstr!(PathPackageRef);

impl PackageRef for PathPackageRef {
	fn structure_kind(&self) -> StructureKind {
		StructureKind::PesdeV1
	}
}

impl std::fmt::Display for PathPackageRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.path.display())
	}
}

impl std::str::FromStr for PathPackageRef {
	type Err = std::convert::Infallible;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(PathPackageRef {
			path: PathBuf::from(s),
		})
	}
}
