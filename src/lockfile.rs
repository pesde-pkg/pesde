#![allow(deprecated)]
use crate::{
    manifest::{
        overrides::OverrideKey,
        target::{Target, TargetKind},
        DependencyType,
    },
    names::PackageName,
    source::{
        ids::{PackageId, VersionId},
        refs::PackageRefs,
        specifiers::DependencySpecifiers,
        traits::PackageRef,
    },
};
use relative_path::RelativePathBuf;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// A graph of dependencies
pub type Graph<Node> = BTreeMap<PackageId, Node>;

/// A dependency graph node
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DependencyGraphNode {
    /// The alias, specifier, and original (as in the manifest) type for the dependency, if it is a direct dependency (i.e. used by the current project)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct: Option<(String, DependencySpecifiers, DependencyType)>,
    /// The dependencies of the package
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<PackageId, String>,
    /// The resolved (transformed, for example Peer -> Standard) type of the dependency
    pub resolved_ty: DependencyType,
    /// Whether the resolved type should be Peer if this isn't depended on
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_peer: bool,
    /// The package reference
    pub pkg_ref: PackageRefs,
}

impl DependencyGraphNode {
    pub(crate) fn base_folder(&self, version_id: &VersionId, project_target: TargetKind) -> String {
        if self.pkg_ref.use_new_structure() {
            version_id.target().packages_folder(&project_target)
        } else {
            "..".to_string()
        }
    }

    /// Returns the folder to store the contents of the package in
    pub fn container_folder<P: AsRef<Path>>(&self, path: &P, package_id: &PackageId) -> PathBuf {
        let (name, version) = package_id.parts();

        if self.pkg_ref.like_wally() {
            return path
                .as_ref()
                .join(format!(
                    "{}_{}@{}",
                    package_id.name().as_str().0,
                    name.as_str().1,
                    version
                ))
                .join(name.as_str().1);
        }

        path.as_ref()
            .join(name.escaped())
            .join(version.to_string())
            .join(name.as_str().1)
    }
}

/// A graph of `DependencyGraphNode`s
pub type DependencyGraph = Graph<DependencyGraphNode>;

/// A downloaded dependency graph node, i.e. a `DependencyGraphNode` with a `Target`
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DownloadedDependencyGraphNode {
    /// The target of the package
    pub target: Target,
    /// The node
    #[serde(flatten)]
    pub node: DependencyGraphNode,
}

/// A graph of `DownloadedDependencyGraphNode`s
pub type DownloadedGraph = Graph<DownloadedDependencyGraphNode>;

/// A lockfile
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Lockfile {
    /// The name of the package
    pub name: PackageName,
    /// The version of the package
    pub version: Version,
    /// The target of the package
    pub target: TargetKind,
    /// The overrides of the package
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<OverrideKey, DependencySpecifiers>,

    /// The workspace members
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub workspace: BTreeMap<PackageName, BTreeMap<TargetKind, RelativePathBuf>>,

    /// The graph of dependencies
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub graph: DownloadedGraph,
}

/// Old lockfile stuff. Will be removed in a future version.
#[deprecated(
    note = "Intended to be used to migrate old lockfiles to the new format. Will be removed in a future version."
)]
pub mod old {
    use crate::{
        manifest::{
            overrides::OverrideKey,
            target::{Target, TargetKind},
            DependencyType,
        },
        names::{PackageName, PackageNames},
        source::{
            ids::{PackageId, VersionId},
            refs::PackageRefs,
            specifiers::DependencySpecifiers,
        },
    };
    use relative_path::RelativePathBuf;
    use semver::Version;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    /// An old dependency graph node
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct DependencyGraphNodeOld {
        /// The alias, specifier, and original (as in the manifest) type for the dependency, if it is a direct dependency (i.e. used by the current project)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub direct: Option<(String, DependencySpecifiers, DependencyType)>,
        /// The dependencies of the package
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pub dependencies: BTreeMap<PackageNames, (VersionId, String)>,
        /// The resolved (transformed, for example Peer -> Standard) type of the dependency
        pub resolved_ty: DependencyType,
        /// Whether the resolved type should be Peer if this isn't depended on
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub is_peer: bool,
        /// The package reference
        pub pkg_ref: PackageRefs,
    }

    /// A downloaded dependency graph node, i.e. a `DependencyGraphNode` with a `Target`
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct DownloadedDependencyGraphNodeOld {
        /// The target of the package
        pub target: Target,
        /// The node
        #[serde(flatten)]
        pub node: DependencyGraphNodeOld,
    }

    /// An old version of a lockfile
    #[derive(Serialize, Deserialize, Debug, Clone)]
    pub struct LockfileOld {
        /// The name of the package
        pub name: PackageName,
        /// The version of the package
        pub version: Version,
        /// The target of the package
        pub target: TargetKind,
        /// The overrides of the package
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pub overrides: BTreeMap<OverrideKey, DependencySpecifiers>,

        /// The workspace members
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pub workspace: BTreeMap<PackageName, BTreeMap<TargetKind, RelativePathBuf>>,

        /// The graph of dependencies
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pub graph: BTreeMap<PackageNames, BTreeMap<VersionId, DownloadedDependencyGraphNodeOld>>,
    }

    impl LockfileOld {
        /// Converts this lockfile to a new lockfile
        pub fn to_new(self) -> super::Lockfile {
            super::Lockfile {
                name: self.name,
                version: self.version,
                target: self.target,
                overrides: self.overrides,
                workspace: self.workspace,
                graph: self
                    .graph
                    .into_iter()
                    .flat_map(|(name, versions)| {
                        versions.into_iter().map(move |(version, node)| {
                            (
                                PackageId(name.clone(), version),
                                super::DownloadedDependencyGraphNode {
                                    target: node.target,
                                    node: super::DependencyGraphNode {
                                        direct: node.node.direct,
                                        dependencies: node
                                            .node
                                            .dependencies
                                            .into_iter()
                                            .map(|(name, (version, alias))| {
                                                (PackageId(name, version), alias)
                                            })
                                            .collect(),
                                        resolved_ty: node.node.resolved_ty,
                                        is_peer: node.node.is_peer,
                                        pkg_ref: node.node.pkg_ref,
                                    },
                                },
                            )
                        })
                    })
                    .collect(),
            }
        }
    }
}
