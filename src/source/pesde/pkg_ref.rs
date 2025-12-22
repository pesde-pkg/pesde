use std::{fmt::Display, str::FromStr};

use crate::{
	names::PackageName,
	ser_display_deser_fromstr,
	source::{PackageRef, refs::StructureKind},
};

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

impl FromStr for PesdePackageRef {
	type Err = crate::names::errors::PackageNameError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(PesdePackageRef { name: s.parse()? })
	}
}
