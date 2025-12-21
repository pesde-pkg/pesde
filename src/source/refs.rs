use crate::{
	manifest::{Alias, DependencyType},
	source::{PackageSources, pesde, specifiers::DependencySpecifiers, traits::PackageRef},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A type of structure
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StructureKind {
	/// Linker files in the parent of the folder containing the package's contents
	Wally,
	/// `packages` folders inside the package's content folder
	PesdeV1,
}

/// All possible package references
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case", tag = "ref_ty")]
pub enum PackageRefs {
	/// A pesde package reference
	Pesde(pesde::pkg_ref::PesdePackageRef),
	/// A Wally package reference
	#[cfg(feature = "wally-compat")]
	Wally(crate::source::wally::pkg_ref::WallyPackageRef),
	/// A Git package reference
	Git(crate::source::git::pkg_ref::GitPackageRef),
	/// A path package reference
	Path(crate::source::path::pkg_ref::PathPackageRef),
}

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
	fn dependencies(&self) -> &BTreeMap<Alias, (DependencySpecifiers, DependencyType)> {
		match self {
			PackageRefs::Pesde(pkg_ref) => pkg_ref.dependencies(),
			#[cfg(feature = "wally-compat")]
			PackageRefs::Wally(pkg_ref) => pkg_ref.dependencies(),
			PackageRefs::Git(pkg_ref) => pkg_ref.dependencies(),
			PackageRefs::Path(pkg_ref) => pkg_ref.dependencies(),
		}
	}

	fn structure_kind(&self) -> StructureKind {
		match self {
			PackageRefs::Pesde(pkg_ref) => pkg_ref.structure_kind(),
			#[cfg(feature = "wally-compat")]
			PackageRefs::Wally(pkg_ref) => pkg_ref.structure_kind(),
			PackageRefs::Git(pkg_ref) => pkg_ref.structure_kind(),
			PackageRefs::Path(pkg_ref) => pkg_ref.structure_kind(),
		}
	}

	fn source(&self) -> PackageSources {
		match self {
			PackageRefs::Pesde(pkg_ref) => pkg_ref.source(),
			#[cfg(feature = "wally-compat")]
			PackageRefs::Wally(pkg_ref) => pkg_ref.source(),
			PackageRefs::Git(pkg_ref) => pkg_ref.source(),
			PackageRefs::Path(pkg_ref) => pkg_ref.source(),
		}
	}
}
