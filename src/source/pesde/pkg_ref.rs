use std::fmt::Display;
use std::str::FromStr;

use crate::names::PackageName;
use crate::ser_display_deser_fromstr;
use crate::source::PackageRef;
use crate::source::StructureKind;

/// A pesde package reference
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PesdePackageRef {
	/// The name of the package
	pub name: PackageName,
}
ser_display_deser_fromstr!(PesdePackageRef);

impl PackageRef for PesdePackageRef {
	fn structure_kind(&self) -> StructureKind {
		StructureKind::PesdeV1
	}
}

impl Display for PesdePackageRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.name)
	}
}

/// Errors that can occur when parsing a pesde package reference
pub type PesdePackageRefParseError = crate::names::errors::PackageNameError;

impl FromStr for PesdePackageRef {
	type Err = PesdePackageRefParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(PesdePackageRef { name: s.parse()? })
	}
}
