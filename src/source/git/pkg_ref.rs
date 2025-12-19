use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
	manifest::{Alias, DependencyType},
	source::{
		DependencySpecifiers, PackageRef, PackageSources, git::GitPackageSource,
		refs::StructureKind,
	},
};

/// A Git package reference
#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct GitPackageRef {
	/// The repository of the package
	#[serde(
		serialize_with = "crate::util::serialize_gix_url",
		deserialize_with = "crate::util::deserialize_gix_url"
	)]
	pub repo: gix::Url,
	/// The id of the package's tree
	pub tree_id: String,
	/// The dependencies of the package
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub dependencies: BTreeMap<Alias, (DependencySpecifiers, DependencyType)>,
	/// The structure kind of this package
	pub structure_kind: StructureKind,
}
impl PackageRef for GitPackageRef {
	fn dependencies(&self) -> &BTreeMap<Alias, (DependencySpecifiers, DependencyType)> {
		&self.dependencies
	}

	fn structure_kind(&self) -> StructureKind {
		self.structure_kind
	}

	fn source(&self) -> PackageSources {
		PackageSources::Git(GitPackageSource::new(self.repo.clone()))
	}
}
