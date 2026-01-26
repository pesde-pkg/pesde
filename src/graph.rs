use crate::{
	Importer, PACKAGES_CONTAINER_NAME,
	manifest::{Alias, DependencyType, overrides::OverrideKey},
	source::{
		ids::PackageId, refs::StructureKind, specifiers::DependencySpecifiers,
		traits::PackageRef as _,
	},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// A dependency graph importer
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DependencyGraphImporter {
	/// The dependencies of the importer
	pub dependencies: BTreeMap<Alias, (PackageId, DependencySpecifiers, DependencyType)>,
	/// The overrides of the importer
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub overrides: BTreeMap<OverrideKey, DependencySpecifiers>,
}

/// A graph of dependencies in a project
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DependencyGraph {
	/// The importers in the graph
	pub importers: BTreeMap<Importer, DependencyGraphImporter>,
	/// The nodes in the graph
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub nodes: BTreeMap<PackageId, DependencyGraphNode>,
}

/// A dependency graph node
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DependencyGraphNode {
	/// The dependencies of the package
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub dependencies: BTreeMap<Alias, PackageId>,
	/// The checksum of the package
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub checksum: String,
}

impl DependencyGraphNode {
	pub(crate) fn dependencies_dir(package_id: &PackageId) -> &'static str {
		match package_id.pkg_ref().structure_kind() {
			StructureKind::Wally => "..",
			StructureKind::PesdeV1 => package_id.v_id().target().packages_dir(),
		}
	}

	/// Returns the directory to store the contents of the package in
	#[must_use]
	pub fn container_dir(package_id: &PackageId) -> PathBuf {
		PathBuf::from(package_id.escaped()).join(package_id.v_id().escaped())
	}

	/// Returns the directory to store the contents of the package in starting from the project's package directory
	#[must_use]
	pub fn container_dir_top_level(package_id: &PackageId) -> PathBuf {
		PathBuf::from(package_id.v_id().target().packages_dir())
			.join(PACKAGES_CONTAINER_NAME)
			.join(Self::container_dir(package_id))
	}
}

/// A graph of [`DependencyType`]s, used for peer dependency warnings
#[derive(Debug)]
pub struct DependencyTypeGraph {
	/// The importers in the graph
	pub importers: BTreeMap<Importer, BTreeMap<Alias, PackageId>>,
	/// The nodes in the graph
	pub nodes: BTreeMap<PackageId, DependencyTypeGraphNode>,
}

/// A dependency graph node for type information
#[derive(Debug)]
pub struct DependencyTypeGraphNode {
	/// The dependencies of the package
	pub dependencies: BTreeMap<Alias, (PackageId, DependencyType)>,
}
