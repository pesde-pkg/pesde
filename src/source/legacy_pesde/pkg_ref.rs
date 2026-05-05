//! Legacy pesde package reference
use std::fmt::Display;
use std::str::FromStr;

use crate::names::PackageName;
use crate::ser_display_deser_fromstr;
use crate::source::PackageRef;
use crate::source::legacy_pesde::target::TargetKind;

/// A legacy pesde package reference
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LegacyPesdePackageRef {
	/// The name of the package
	pub name: PackageName,
	/// The target of the package
	pub target: TargetKind,
}
ser_display_deser_fromstr!(LegacyPesdePackageRef);

impl PackageRef for LegacyPesdePackageRef {}

impl Display for LegacyPesdePackageRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}+{}", self.name, self.target)
	}
}

/// Errors that can occur when parsing a legacy pesde package reference
pub type LegacyPesdePackageRefParseError = errors::LegacyPesdePackageRefParseError;

impl FromStr for LegacyPesdePackageRef {
	type Err = LegacyPesdePackageRefParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let Some((name, target)) = s.split_once('+') else {
			return Err(errors::LegacyPesdePackageRefParseErrorKind::InvalidFormat.into());
		};

		Ok(LegacyPesdePackageRef {
			name: name.parse()?,
			target: target.parse()?,
		})
	}
}

/// Error that can occur when parsing a legacy pesde package reference from a string
pub mod errors {
	use crate::names::errors::PackageNameError;
	use thiserror::Error;

	/// Error that can occur when parsing a legacy pesde package reference from a string
	#[allow(clippy::enum_variant_names)]
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = LegacyPesdePackageRefParseError))]
	pub enum LegacyPesdePackageRefParseErrorKind {
		/// The format of the package reference is invalid
		#[error("invalid format for pesde package reference")]
		InvalidFormat,

		/// The package name is invalid
		#[error("invalid package name")]
		InvalidPackageName(#[from] PackageNameError),

		/// The target is invalid
		#[error("invalid target")]
		InvalidTarget(#[from] crate::source::legacy_pesde::target::errors::TargetKindFromStr),
	}
}
