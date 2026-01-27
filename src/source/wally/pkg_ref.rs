use std::fmt::Display;
use std::str::FromStr;

use crate::names::wally::WallyPackageName;
use crate::ser_display_deser_fromstr;
use crate::source::PackageRef;
use crate::source::refs::StructureKind;

/// A Wally package reference
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WallyPackageRef {
	/// The name of the package
	pub name: WallyPackageName,
}
ser_display_deser_fromstr!(WallyPackageRef);

impl PackageRef for WallyPackageRef {
	fn structure_kind(&self) -> StructureKind {
		StructureKind::Wally
	}
}

impl Display for WallyPackageRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.name)
	}
}

impl FromStr for WallyPackageRef {
	type Err = crate::names::errors::WallyPackageNameError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(WallyPackageRef { name: s.parse()? })
	}
}
