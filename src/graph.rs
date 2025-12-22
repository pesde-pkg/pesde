use crate::{
	PACKAGES_CONTAINER_NAME, Project,
	manifest::{
		Alias, DependencyType,
		target::{Target, TargetKind},
	},
	source::{
		ids::PackageId, refs::StructureKind, specifiers::DependencySpecifiers,
		traits::PackageRef as _,
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
	pub direct: Option<(Alias, DependencySpecifiers, DependencyType)>,
	/// The dependencies of the package
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub dependencies: BTreeMap<Alias, PackageId>,
}

impl DependencyGraphNode {
	pub(crate) fn dependencies_dir(
		package_id: &PackageId,
		project_target: TargetKind,
	) -> &'static str {
		match package_id.pkg_ref().structure_kind() {
			StructureKind::Wally => "..",
			StructureKind::PesdeV1 => package_id.v_id().target().packages_folder(project_target),
		}
	}

	/// Returns the folder to store the contents of the package in
	#[must_use]
	pub fn container_folder(&self, package_id: &PackageId) -> PathBuf {
		PathBuf::from(package_id.escaped()).join(package_id.v_id().escaped())
	}

	/// Returns the folder to store the contents of the package in starting from the project's package directory
	#[must_use]
	pub fn container_folder_from_project(
		&self,
		package_id: &PackageId,
		project: &Project,
		manifest_target_kind: TargetKind,
	) -> PathBuf {
		project
			.package_dir()
			.join(manifest_target_kind.packages_folder(package_id.v_id().target()))
			.join(PACKAGES_CONTAINER_NAME)
			.join(self.container_folder(package_id))
	}
}

/// A graph of `DependencyGraphNode`s
pub type DependencyGraph = Graph<DependencyGraphNode>;

/// A dependency graph node for type information
#[derive(Debug)]
pub struct DependencyTypeGraphNode {
	/// The alias of the dependency, if it is a direct dependency
	pub direct: Option<Alias>,
	/// The dependencies of the package
	pub dependencies: BTreeMap<Alias, (PackageId, DependencyType)>,
}

/// A graph of `DependencyGraphTypesNode`s
pub type TypeGraph = Graph<DependencyTypeGraphNode>;

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
