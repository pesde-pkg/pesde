//! Lockfile
use std::collections::BTreeMap;

use crate::graph::DependencyGraph;
use crate::source::PackageSources;
use crate::source::SourceState;
use serde::Deserialize;
use serde::Serialize;

/// The current format of the lockfile
pub const CURRENT_FORMAT: usize = 3;

/// A lockfile
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Lockfile {
	/// State stored by sources
	#[serde(
		default,
		skip_serializing_if = "should_skip_source_states",
		serialize_with = "serialize_source_states"
	)]
	pub source_states: BTreeMap<PackageSources, SourceState>,
	/// The graph of dependencies
	pub graph: DependencyGraph,
}

fn should_skip_source_states(source_states: &BTreeMap<PackageSources, SourceState>) -> bool {
	source_states
		.values()
		.all(|state| !matches!(state, SourceState::Pesde(_)))
}

fn serialize_source_states<S>(
	source_states: &BTreeMap<PackageSources, SourceState>,
	serializer: S,
) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	let source_states = source_states
		.iter()
		// other sources do not have any state, the toml crate will reject the sealing unit types
		.filter(|(_, state)| matches!(state, SourceState::Pesde(_)))
		.collect::<BTreeMap<_, _>>();
	source_states.serialize(serializer)
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
		format if format < CURRENT_FORMAT => {
			Err(errors::ParseLockfileErrorKind::TooOld(format).into())
		}
		format => Err(errors::ParseLockfileErrorKind::TooNew(format).into()),
	}
}

/// Errors that can occur when working with lockfiles
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when parsing a lockfile
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = ParseLockfileError))]
	#[non_exhaustive]
	pub enum ParseLockfileErrorKind {
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
