#![allow(deprecated)]
use crate::{
	graph::DependencyGraph,
	manifest::{overrides::OverrideKey, target::TargetKind},
	names::PackageName,
	source::{ids::PackageId, specifiers::DependencySpecifiers},
};
use relative_path::RelativePathBuf;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt::Debug};

/// The current format of the lockfile
pub const CURRENT_FORMAT: usize = 1;

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

/// Parses the lockfile, updating it to the [`CURRENT_FORMAT`] from the format it's at
pub fn parse_lockfile(lockfile: &str) -> Result<Lockfile, errors::ParseLockfileError> {
	#[derive(Serialize, Deserialize, Debug)]
	pub struct LockfileFormat {
		#[serde(default)]
		pub format: usize,
	}

	let format: LockfileFormat = toml::de::from_str(lockfile)?;
	let format = format.format;

	match format {
		0 => {
			let this = match toml::from_str(lockfile) {
				Ok(lockfile) => return Ok(lockfile),
				Err(e) => match toml::from_str::<v0::Lockfile>(lockfile) {
					Ok(this) => this,
					Err(_) => return Err(errors::ParseLockfileError::De(e)),
				},
			};

			Ok(Lockfile {
				name: this.name,
				version: this.version,
				target: this.target,
				overrides: this.overrides,
				workspace: this.workspace,
				graph: this
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
			})
		}
		CURRENT_FORMAT => toml::de::from_str(lockfile).map_err(Into::into),
		format => Err(errors::ParseLockfileError::TooNew(format)),
	}
}

/// Lockfile v0
pub mod v0 {
	use crate::{
		manifest::{
			overrides::OverrideKey,
			target::{Target, TargetKind},
			Alias, DependencyType,
		},
		names::{PackageName, PackageNames},
		source::{ids::VersionId, refs::PackageRefs, specifiers::DependencySpecifiers},
	};
	use relative_path::RelativePathBuf;
	use semver::Version;
	use serde::{Deserialize, Serialize};
	use std::collections::BTreeMap;

	/// A dependency graph node
	#[derive(Serialize, Deserialize, Debug, Clone)]
	pub struct DependencyGraphNode {
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
	pub struct DownloadedDependencyGraphNode {
		/// The target of the package
		pub target: Target,
		/// The node
		#[serde(flatten)]
		pub node: DependencyGraphNode,
	}

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
		pub graph: BTreeMap<PackageNames, BTreeMap<VersionId, DownloadedDependencyGraphNode>>,
	}
}

/// Errors that can occur when working with lockfiles
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when parsing a lockfile
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum ParseLockfileError {
		/// The lockfile format is too new
		#[error("lockfile format {} is too new. newest supported format: {}", .0, super::CURRENT_FORMAT)]
		TooNew(usize),

		/// Deserializing the lockfile failed
		#[error("deserializing the lockfile failed")]
		De(#[from] toml::de::Error),
	}
}
