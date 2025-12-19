use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
	manifest::{Alias, DependencyType},
	source::{
		DependencySpecifiers, PackageRef, PackageSources, pesde::PesdePackageSource,
		refs::StructureKind,
	},
	GixUrl,
};

/// A pesde package reference
#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct PesdePackageRef {
	/// The index of the package
	pub index_url: GixUrl,
	/// The dependencies of the package
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub dependencies: BTreeMap<Alias, (DependencySpecifiers, DependencyType)>,
}
impl PackageRef for PesdePackageRef {
	fn dependencies(&self) -> &BTreeMap<Alias, (DependencySpecifiers, DependencyType)> {
		&self.dependencies
	}

	fn structure_kind(&self) -> StructureKind {
		StructureKind::PesdeV1
	}

	fn source(&self) -> PackageSources {
		PackageSources::Pesde(PesdePackageSource::new(self.index_url.clone()))
	}
}
