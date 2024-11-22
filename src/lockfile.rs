use crate::{
    manifest::{
        overrides::OverrideKey,
        target::{Target, TargetKind},
        DependencyType,
    },
    names::{PackageName, PackageNames},
    source::{
        refs::PackageRefs, specifiers::DependencySpecifiers, traits::PackageRef,
        version_id::VersionId,
    },
};
use relative_path::RelativePathBuf;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::{btree_map::Entry, BTreeMap},
    path::{Path, PathBuf},
};

/// A graph of dependencies
pub type Graph<Node> = BTreeMap<PackageNames, BTreeMap<VersionId, Node>>;

/// A dependency graph node
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DependencyGraphNode {
    /// The alias, specifier, and original (as in the manifest) type for the dependency, if it is a direct dependency (i.e. used by the current project)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct: Option<(String, DependencySpecifiers, DependencyType)>,
    /// The dependencies of the package
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<PackageNames, (VersionId, String)>,
    /// The resolved (transformed, for example Peer -> Standard) type of the dependency
    pub resolved_ty: DependencyType,
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
    pub fn container_folder<P: AsRef<Path>>(
        &self,
        path: &P,
        name: &PackageNames,
        version: &Version,
    ) -> PathBuf {
        path.as_ref()
            .join(name.escaped())
            .join(version.to_string())
            .join(name.as_str().1)
    }
}

/// A graph of `DependencyGraphNode`s
pub type DependencyGraph = Graph<DependencyGraphNode>;

pub(crate) fn insert_node(
    graph: &mut DependencyGraph,
    name: PackageNames,
    version: VersionId,
    mut node: DependencyGraphNode,
    is_top_level: bool,
) {
    if !is_top_level && node.direct.take().is_some() {
        log::debug!(
            "tried to insert {name}@{version} as direct dependency from a non top-level context",
        );
    }

    match graph
        .entry(name.clone())
        .or_default()
        .entry(version.clone())
    {
        Entry::Vacant(entry) => {
            entry.insert(node);
        }
        Entry::Occupied(existing) => {
            let current_node = existing.into_mut();

            match (&current_node.direct, &node.direct) {
                (Some(_), Some(_)) => {
                    log::warn!("duplicate direct dependency for {name}@{version}");
                }

                (None, Some(_)) => {
                    current_node.direct = node.direct;
                }

                (_, _) => {}
            }
        }
    }
}

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
