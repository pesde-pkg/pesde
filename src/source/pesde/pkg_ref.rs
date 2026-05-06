//! pesde package reference
use std::fmt::Display;
use std::str::FromStr;

use crate::names::PackageName;
use crate::ser_display_deser_fromstr;
use crate::source::PackageRef;

/// A pesde package reference
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PesdePackageRef {
	/// The name of the package
	pub name: PackageName,
}
ser_display_deser_fromstr!(PesdePackageRef);

impl PackageRef for PesdePackageRef {}

impl Display for PesdePackageRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.name)
	}
}

/// Errors that can occur when parsing a pesde package reference
pub type PesdePackageRefParseError = errors::PesdePackageRefParseError;

impl FromStr for PesdePackageRef {
	type Err = PesdePackageRefParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(PesdePackageRef { name: s.parse()? })
	}
}

/// Error that can occur when parsing a pesde package reference from a string
pub mod errors {
	use crate::names::errors::PackageNameError;
	use thiserror::Error;

	/// Error that can occur when parsing a pesde package reference from a string
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = PesdePackageRefParseError))]
	pub enum PesdePackageRefParseErrorKind {
		/// The package name is invalid
		#[error("invalid package name")]
		InvalidPackageName(#[from] PackageNameError),
	}
}
