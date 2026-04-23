//! The dependency graph
use std::collections::BTreeMap;
use std::collections::HashSet;

use serde::Deserialize;
use serde::Serialize;

use crate::Importer;
use crate::hash::Hash;
use crate::manifest::Alias;
use crate::manifest::DependencyType;
use crate::source::DependencySpecifier as _;
use crate::source::DependencySpecifiers;
use crate::source::Realm;
use crate::source::ResolvedPackage;
use crate::source::StructureKind;
use crate::source::ids::PackageId;

/// A dependency graph importer
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DependencyGraphImporter {
	/// The dependencies of the importer
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub dependencies: BTreeMap<Alias, (PackageId, DependencySpecifiers, DependencyType)>,
}

/// A dependency graph node dependency
#[derive(Debug, Clone)]
pub struct DependencyGraphNodeDependency {
	/// The package ID of the dependency
	pub id: PackageId,
	/// The type of the dependency
	pub ty: DependencyType,
	/// The realm of the dependency
	pub realm: Option<Realm>,
}

// serialized as a tuple for increased readability in the lockfile since the toml crate doesn't support forcing inline tables
// can't simply use a tuple struct because of the optional realm field, which the toml crate doesn't support

impl Serialize for DependencyGraphNodeDependency {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		if let Some(realm) = &self.realm {
			(&self.id, &self.ty, realm).serialize(serializer)
		} else {
			(&self.id, &self.ty).serialize(serializer)
		}
	}
}

impl<'de> Deserialize<'de> for DependencyGraphNodeDependency {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct Visitor;

		impl<'de> serde::de::Visitor<'de> for Visitor {
			type Value = DependencyGraphNodeDependency;

			fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
				formatter.write_str(
					"a tuple of (PackageId, DependencyType) or (PackageId, DependencyType, Realm)",
				)
			}

			fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
			where
				A: serde::de::SeqAccess<'de>,
			{
				let id: PackageId = seq
					.next_element()?
					.ok_or_else(|| serde::de::Error::invalid_length(0, &"2 or 3 elements"))?;

				let ty: DependencyType = seq
					.next_element()?
					.ok_or_else(|| serde::de::Error::invalid_length(1, &"2 or 3 elements"))?;

				let realm: Option<Realm> = seq.next_element()?;

				Ok(DependencyGraphNodeDependency { id, ty, realm })
			}
		}

		deserializer.deserialize_seq(Visitor)
	}
}

/// A dependency graph node
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DependencyGraphNode {
	/// The dependencies of the package
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub dependencies: BTreeMap<Alias, DependencyGraphNodeDependency>,
	/// The checksum of the package
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub checksum: Option<Hash>,
	/// The structure kind of the package
	pub structure_kind: StructureKind,
}

/// A graph of dependencies in a project
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DependencyGraph {
	/// The importers in the graph
	pub importers: BTreeMap<Importer, DependencyGraphImporter>,
	/// The overrides in this workspace
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub overrides: BTreeMap<PackageId, DependencySpecifiers>,
	/// The nodes in the graph
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub nodes: BTreeMap<PackageId, DependencyGraphNode>,
}

impl DependencyGraph {
	/// Returns the resolved realm of a package
	/// Resolution works as so:
	/// if the package is never depended on with a realm, it has no realm
	/// if the package is depended on as a shared dependency anywhere in the graph, it is a shared package
	/// otherwise, it is a server package
	#[must_use]
	pub fn realm_of(&self, importer: &Importer, package_id: &PackageId) -> Option<Realm> {
		let graph_importer = self.importers.get(importer)?;

		let mut visited = HashSet::new();
		let mut ret = None;
		let mut queue = graph_importer
			.dependencies
			.values()
			.filter_map(|(id, spec, _)| spec.realm().map(|realm| (id, realm)))
			.collect::<Vec<_>>();

		while let Some((pkg_id, realm)) = queue.pop() {
			if pkg_id == package_id {
				match realm {
					Realm::Shared => return Some(Realm::Shared),
					Realm::Server => ret = Some(Realm::Server),
				}
			}

			if let Some(node) = self.nodes.get(pkg_id)
				&& visited.insert(pkg_id)
			{
				for dep in node.dependencies.values() {
					let Some(realm) = dep.realm else {
						continue;
					};
					queue.push((&dep.id, realm));
				}
			}
		}

		ret
	}

	/// Returns the resolved package for a given package ID, if it exists in the graph
	#[must_use]
	pub fn resolved_package(&self, package_id: &PackageId) -> Option<ResolvedPackage> {
		self.nodes.get(package_id).map(|node| ResolvedPackage {
			id: package_id.clone(),
			structure_kind: node.structure_kind.clone(),
		})
	}
}
