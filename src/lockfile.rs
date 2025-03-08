#![allow(deprecated)]
use crate::{
	graph::DependencyGraph,
	manifest::{overrides::OverrideKey, target::TargetKind},
	names::PackageName,
	source::specifiers::DependencySpecifiers,
};
use relative_path::RelativePathBuf;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
	#[serde(default, skip_serializing_if = "DependencyGraph::is_empty")]
	pub graph: DependencyGraph,
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
			Alias, DependencyType,
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
		pub direct: Option<(Alias, DependencySpecifiers, DependencyType)>,
		/// The dependencies of the package
		#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
		pub dependencies: BTreeMap<PackageNames, (VersionId, Alias)>,
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
		#[must_use]
		#[allow(clippy::wrong_self_convention)]
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
								crate::graph::DependencyGraphNode {
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
							)
						})
					})
					.collect(),
			}
		}
	}
}
