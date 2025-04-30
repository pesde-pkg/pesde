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

/// The current format of the lockfile
pub const CURRENT_FORMAT: usize = 2;

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
		CURRENT_FORMAT => toml::de::from_str(lockfile).map_err(Into::into),
		format if format < CURRENT_FORMAT => Err(errors::ParseLockfileError::TooOld(format)),
		format => Err(errors::ParseLockfileError::TooNew(format)),
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

		/// The lockfile format is too old
		#[error("lockfile format {} is too old. manual deletion is required. current format: {}", .0, super::CURRENT_FORMAT)]
		TooOld(usize),

		/// Deserializing the lockfile failed
		#[error("deserializing the lockfile failed")]
		De(#[from] toml::de::Error),
	}
}
