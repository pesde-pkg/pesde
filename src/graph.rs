use crate::{
    manifest::{
        target::{Target, TargetKind},
        DependencyType,
    },
    source::{
        ids::{PackageId, VersionId},
        refs::PackageRefs,
        specifiers::DependencySpecifiers,
        traits::PackageRef,
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

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
    pub fn container_folder(&self, package_id: &PackageId) -> PathBuf {
        let (name, version) = package_id.parts();

        if self.pkg_ref.is_wally_package() {
            return PathBuf::from(format!(
                "{}_{}@{}",
                name.as_str().0,
                name.as_str().1,
                version
            ))
            .join(name.as_str().1);
        }

        PathBuf::from(name.escaped())
            .join(version.to_string())
            .join(name.as_str().1)
    }
}

/// A graph of `DependencyGraphNode`s
pub type DependencyGraph = Graph<DependencyGraphNode>;

/// A dependency graph node with a `Target`
#[derive(Debug, Clone)]
pub struct DependencyGraphNodeWithTarget {
    /// The target of the package
    pub target: Target,
    /// The node
    pub node: DependencyGraphNode,
}

/// A graph of `DownloadedDependencyGraphNode`s
pub type DependencyGraphWithTarget = Graph<DependencyGraphNodeWithTarget>;

/// A trait for converting a graph to a different type of graph
pub trait ConvertableGraph<Node> {
    /// Converts the graph to a different type of graph
    fn convert(self) -> Graph<Node>;
}

impl ConvertableGraph<DependencyGraphNode> for DependencyGraphWithTarget {
    fn convert(self) -> Graph<DependencyGraphNode> {
        self.into_iter().map(|(id, node)| (id, node.node)).collect()
    }
}
