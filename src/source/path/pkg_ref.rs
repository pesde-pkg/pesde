use crate::{
    manifest::DependencyType,
    source::{path::PathPackageSource, DependencySpecifiers, PackageRef, PackageSources},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// A path package reference
#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct PathPackageRef {
    /// The path of the package
    pub path: PathBuf,
    /// The dependencies of the package
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, (DependencySpecifiers, DependencyType)>,
}
impl PackageRef for PathPackageRef {
    fn dependencies(&self) -> &BTreeMap<String, (DependencySpecifiers, DependencyType)> {
        &self.dependencies
    }

    fn use_new_structure(&self) -> bool {
        true
    }

    fn source(&self) -> PackageSources {
        PackageSources::Path(PathPackageSource)
    }
}
