use crate::{
	ser_display_deser_fromstr,
	source::{PackageRef, path::RelativeOrAbsolutePath, refs::StructureKind},
};

/// A path package reference
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathPackageRef {
	/// The path of the package
	pub path: RelativeOrAbsolutePath,
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

impl std::str::FromStr for PathPackageRef {
	type Err = std::convert::Infallible;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(PathPackageRef {
			path: s.parse().unwrap(),
		})
	}
}
