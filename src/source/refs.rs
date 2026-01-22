#![expect(deprecated)]
use std::{fmt::Display, str::FromStr};

use crate::{ser_display_deser_fromstr, source::traits::PackageRef};

/// A type of structure
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StructureKind {
	/// Linker files in the parent of the directory containing the package's contents
	Wally,
	/// `*_packages` directories inside the package's content directory
	PesdeV1,
}

impl Display for StructureKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			StructureKind::Wally => write!(f, "wally"),
			StructureKind::PesdeV1 => write!(f, "pesde_v1"),
		}
	}
}

impl FromStr for StructureKind {
	type Err = errors::StructureKindParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"wally" => Ok(StructureKind::Wally),
			"pesde_v1" => Ok(StructureKind::PesdeV1),
			_ => Err(errors::StructureKindParseErrorKind::UnknownKind(s.to_string()).into()),
		}
	}
}

/// All possible package references
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackageRefs {
	/// A pesde package reference
	Pesde(crate::source::pesde::pkg_ref::PesdePackageRef),
	/// A Wally package reference
	#[cfg(feature = "wally-compat")]
	Wally(crate::source::wally::pkg_ref::WallyPackageRef),
	/// A Git package reference
	Git(crate::source::git::pkg_ref::GitPackageRef),
	/// A path package reference
	Path(crate::source::path::pkg_ref::PathPackageRef),
}
ser_display_deser_fromstr!(PackageRefs);

impl PackageRefs {
	/// Returns whether this package reference should be treated as a Wally package
	#[must_use]
	pub fn is_wally_package(&self) -> bool {
		match self {
			#[cfg(feature = "wally-compat")]
			PackageRefs::Wally(_) => true,
			PackageRefs::Git(git) => git.structure_kind() == StructureKind::Wally,
			_ => false,
		}
	}

	/// Returns whether this package reference is local
	#[must_use]
	pub fn is_local(&self) -> bool {
		matches!(self, PackageRefs::Path(_))
	}
}

impl PackageRef for PackageRefs {
	fn structure_kind(&self) -> StructureKind {
		match self {
			PackageRefs::Pesde(pkg_ref) => pkg_ref.structure_kind(),
			#[cfg(feature = "wally-compat")]
			PackageRefs::Wally(pkg_ref) => pkg_ref.structure_kind(),
			PackageRefs::Git(pkg_ref) => pkg_ref.structure_kind(),
			PackageRefs::Path(pkg_ref) => pkg_ref.structure_kind(),
		}
	}
}

impl Display for PackageRefs {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			PackageRefs::Pesde(pkg_ref) => write!(f, "pesde:{pkg_ref}"),
			#[cfg(feature = "wally-compat")]
			PackageRefs::Wally(pkg_ref) => write!(f, "wally:{pkg_ref}"),
			PackageRefs::Git(pkg_ref) => write!(f, "git:{pkg_ref}"),
			PackageRefs::Path(pkg_ref) => write!(f, "path:{pkg_ref}"),
		}
	}
}

impl FromStr for PackageRefs {
	type Err = errors::PackageRefParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (source, pkg_ref) = s
			.split_once(':')
			.ok_or(errors::PackageRefParseErrorKind::InvalidFormat)?;

		match source {
			"pesde" => Ok(PackageRefs::Pesde(pkg_ref.parse()?)),
			"wally" => Ok(PackageRefs::Wally(pkg_ref.parse()?)),
			"git" => Ok(PackageRefs::Git(pkg_ref.parse()?)),
			"path" => Ok(PackageRefs::Path(pkg_ref.parse().unwrap())),
			_ => Err(errors::PackageRefParseErrorKind::UnknownSource(source.to_string()).into()),
		}
	}
}

/// Errors related to package references
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when parsing a structure kind
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = StructureKindParseError))]
	pub enum StructureKindParseErrorKind {
		/// The structure kind is unknown
		#[error("unknown structure kind {0}")]
		UnknownKind(String),
	}

	/// Errors that can occur when parsing a Git package reference
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GitPackageRefParseError))]
	pub enum GitPackageRefParseErrorKind {
		/// The format of the Git package reference is invalid
		#[error("invalid Git package reference format")]
		InvalidFormat,

		/// An error occurred while parsing the structure kind
		#[error("failed to parse structure kind")]
		StructureKindParseError(#[from] StructureKindParseError),
	}

	/// Errors that can occur when parsing a package reference
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = PackageRefParseError))]
	pub enum PackageRefParseErrorKind {
		/// The format of the package reference is invalid
		#[error("invalid package reference format")]
		InvalidFormat,

		/// The source of the package reference is unknown
		#[error("unknown package reference source {0}")]
		UnknownSource(String),

		/// An error occurred while parsing a Pesde package reference
		#[error("failed to parse Pesde package reference")]
		PesdePackageRef(#[from] crate::names::errors::PackageNameError),

		/// An error occurred while parsing a Wally package reference
		#[cfg(feature = "wally-compat")]
		#[error("failed to parse Wally package reference")]
		WallyPackageRef(#[from] crate::names::errors::WallyPackageNameError),

		/// An error occurred while parsing a Git package reference
		#[error("failed to parse Git package reference")]
		GitPackageRef(#[from] GitPackageRefParseError),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn serde_package_refs() {
		let refs = [
			(
				PackageRefs::Pesde("foo/bar".parse().unwrap()),
				"pesde:foo/bar",
			),
			(
				PackageRefs::Wally("foo/bar".parse().unwrap()),
				"wally:foo/bar",
			),
			(
				PackageRefs::Git("abcdef+pesde_v1".parse().unwrap()),
				"git:abcdef+pesde_v1",
			),
			(
				PackageRefs::Path("/dev/null".parse().unwrap()),
				"path:/dev/null",
			),
		];

		for (pkg_ref, serialized) in refs {
			assert_eq!(pkg_ref.to_string(), serialized);
			assert_eq!(PackageRefs::from_str(serialized).unwrap(), pkg_ref);
		}
	}
}
