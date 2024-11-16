use std::collections::BTreeMap;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{
    manifest::{target::Target, DependencyType},
    names::PackageName,
    source::{pesde::PesdePackageSource, DependencySpecifiers, PackageRef, PackageSources},
};

/// A pesde package reference
#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct PesdePackageRef {
    /// The name of the package
    pub name: PackageName,
    /// The version of the package
    pub version: Version,
    /// The index of the package
    #[serde(
        serialize_with = "crate::util::serialize_gix_url",
        deserialize_with = "crate::util::deserialize_gix_url"
    )]
    pub index_url: gix::Url,
    /// The dependencies of the package
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, (DependencySpecifiers, DependencyType)>,
    /// The target of the package
    pub target: Target,
}
impl PackageRef for PesdePackageRef {
    fn dependencies(&self) -> &BTreeMap<String, (DependencySpecifiers, DependencyType)> {
        &self.dependencies
    }

    fn use_new_structure(&self) -> bool {
        true
    }

    fn source(&self) -> PackageSources {
        PackageSources::Pesde(PesdePackageSource::new(self.index_url.clone()))
    }
}
