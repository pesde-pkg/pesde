use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
	manifest::{Alias, DependencyType},
	source::{wally::WallyPackageSource, DependencySpecifiers, PackageRef, PackageSources},
};

/// A Wally package reference
#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct WallyPackageRef {
	/// The index of the package
	#[serde(
		serialize_with = "crate::util::serialize_gix_url",
		deserialize_with = "crate::util::deserialize_gix_url"
	)]
	pub index_url: gix::Url,
	/// The dependencies of the package
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub dependencies: BTreeMap<Alias, (DependencySpecifiers, DependencyType)>,
}
impl PackageRef for WallyPackageRef {
	fn dependencies(&self) -> &BTreeMap<Alias, (DependencySpecifiers, DependencyType)> {
		&self.dependencies
	}

	fn use_new_structure(&self) -> bool {
		false
	}

	fn source(&self) -> PackageSources {
		PackageSources::Wally(WallyPackageSource::new(self.index_url.clone()))
	}
}
