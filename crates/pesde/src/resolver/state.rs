use std::collections::BTreeMap;

use crate::registry::manifest::Alias;
use crate::registry::manifest::DependencyType;
use serde::Deserialize;
use serde::Serialize;

use crate::Importer;
use crate::link::PackageStructure;
use crate::source::DependencySpecifier;
use crate::source::PackageId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolution {
	pub importers: BTreeMap<Importer, ResolutionImporter>,
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub overrides: BTreeMap<PackageId, DependencySpecifier>,
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub graph: BTreeMap<PackageId, DependencyGraphNode>,
}

// TODO: consider handling importer dependencies similar to Cargo, that is not
// storing a DependencySpecifier at all and instead comparing by PackageId/PackageSource

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionImporter {
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub dependencies: BTreeMap<Alias, (PackageId, DependencySpecifier, DependencyType)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyGraphNode {
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub dependencies: BTreeMap<Alias, (PackageId, DependencyType)>,
	/// This is stored here only in-memory as a little cache to avoid redundant lookups.
	/// Serializing it would be leaky and overall a net negative.
	#[serde(default, skip)]
	pub structure: Option<PackageStructure>,
}

pub mod lockfile {
	use thiserror::Error;

	use super::*;

	/// The current format of the lockfile
	pub const CURRENT_FORMAT: u64 = 3;

	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum LockfileParseError {
		#[error("invalid TOML")]
		InvalidToml(#[from] toml::de::Error),

		#[error("lockfile is missing the `format` field")]
		MissingFormatField,

		#[error("lockfile is at format {0}, which cannot be updated to {1}")]
		CannotUpdateFormat(u64, u64),

		#[error(
			"lockfile is at format {0}, which is newer than the format recognized by this version: {1}"
		)]
		NewerFormat(u64, u64),
	}

	pub(crate) fn parse_lockfile(lockfile: &str) -> Result<Resolution, LockfileParseError> {
		let lockfile: toml::value::Table =
			toml::de::from_str(lockfile).map_err(LockfileParseError::InvalidToml)?;
		let format = lockfile
			.get("format")
			.and_then(toml::Value::as_integer)
			.ok_or(LockfileParseError::MissingFormatField)?
			.cast_unsigned();

		match format {
			CURRENT_FORMAT => {
				Resolution::deserialize(lockfile).map_err(LockfileParseError::InvalidToml)
			}
			format if format < CURRENT_FORMAT => Err(LockfileParseError::CannotUpdateFormat(
				format,
				CURRENT_FORMAT,
			)),
			format => Err(LockfileParseError::NewerFormat(format, CURRENT_FORMAT)),
		}
	}
}
