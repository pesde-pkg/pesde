use crate::{
	manifest::{Alias, DependencyType},
	source::{pesde, specifiers::DependencySpecifiers, traits::PackageRef, PackageSources},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
	/// A workspace package reference
	Workspace(crate::source::workspace::pkg_ref::WorkspacePackageRef),
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
			PackageRefs::Git(git) => !git.use_new_structure(),
			_ => false,
		}
	}

	/// Returns whether this package reference is local
	#[must_use]
	pub fn is_local(&self) -> bool {
		matches!(self, PackageRefs::Workspace(_) | PackageRefs::Path(_))
	}
}

impl PackageRef for PackageRefs {
	fn dependencies(&self) -> &BTreeMap<Alias, (DependencySpecifiers, DependencyType)> {
		match self {
			PackageRefs::Pesde(pkg_ref) => pkg_ref.dependencies(),
			#[cfg(feature = "wally-compat")]
			PackageRefs::Wally(pkg_ref) => pkg_ref.dependencies(),
			PackageRefs::Git(pkg_ref) => pkg_ref.dependencies(),
			PackageRefs::Workspace(pkg_ref) => pkg_ref.dependencies(),
			PackageRefs::Path(pkg_ref) => pkg_ref.dependencies(),
		}
	}

	fn use_new_structure(&self) -> bool {
		match self {
			PackageRefs::Pesde(pkg_ref) => pkg_ref.use_new_structure(),
			#[cfg(feature = "wally-compat")]
			PackageRefs::Wally(pkg_ref) => pkg_ref.use_new_structure(),
			PackageRefs::Git(pkg_ref) => pkg_ref.use_new_structure(),
			PackageRefs::Workspace(pkg_ref) => pkg_ref.use_new_structure(),
			PackageRefs::Path(pkg_ref) => pkg_ref.use_new_structure(),
		}
	}

	fn source(&self) -> PackageSources {
		match self {
			PackageRefs::Pesde(pkg_ref) => pkg_ref.source(),
			#[cfg(feature = "wally-compat")]
			PackageRefs::Wally(pkg_ref) => pkg_ref.source(),
			PackageRefs::Git(pkg_ref) => pkg_ref.source(),
			PackageRefs::Workspace(pkg_ref) => pkg_ref.source(),
			PackageRefs::Path(pkg_ref) => pkg_ref.source(),
		}
	}
}
