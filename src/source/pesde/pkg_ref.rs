use std::collections::BTreeMap;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{
    manifest::{DependencyType, Target, TargetKind},
    names::PackageName,
    source::{pesde::PesdePackageSource, DependencySpecifiers, PackageRef, PackageSources},
};

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct PesdePackageRef {
    pub name: PackageName,
    pub version: Version,
    #[serde(
        serialize_with = "crate::util::serialize_gix_url",
        deserialize_with = "crate::util::deserialize_gix_url"
    )]
    pub index_url: gix::Url,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, (DependencySpecifiers, DependencyType)>,
    pub target: Target,
}
impl PackageRef for PesdePackageRef {
    fn dependencies(&self) -> &BTreeMap<String, (DependencySpecifiers, DependencyType)> {
        &self.dependencies
    }

    fn use_new_structure(&self) -> bool {
        true
    }

    fn target_kind(&self) -> TargetKind {
        self.target.kind()
    }

    fn source(&self) -> PackageSources {
        PackageSources::Pesde(PesdePackageSource::new(self.index_url.clone()))
    }
}

impl Ord for PesdePackageRef {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.version.cmp(&other.version)
    }
}

impl PartialOrd for PesdePackageRef {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
