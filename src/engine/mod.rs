/// Sources of engines
pub mod source;

use crate::engine::source::EngineSources;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt::Display, str::FromStr};

/// All supported engines
#[derive(
	SerializeDisplay, DeserializeFromStr, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename_all = "snake_case"))]
pub enum EngineKind {
	/// The pesde package manager
	Pesde,
	/// The Lune runtime
	Lune,
}

impl Display for EngineKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			EngineKind::Pesde => write!(f, "pesde"),
			EngineKind::Lune => write!(f, "lune"),
		}
	}
}

impl FromStr for EngineKind {
	type Err = errors::EngineKindFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"pesde" => Ok(EngineKind::Pesde),
			"lune" => Ok(EngineKind::Lune),
			_ => Err(errors::EngineKindFromStrError::Unknown(s.to_string())),
		}
	}
}

impl EngineKind {
	/// Returns the source to get this engine from
	pub fn source(&self) -> EngineSources {
		match self {
			EngineKind::Pesde => EngineSources::pesde(),
			EngineKind::Lune => EngineSources::lune(),
		}
	}
}

/// Errors related to engine kinds
pub mod errors {
	use thiserror::Error;

	/// Errors which can occur while using the FromStr implementation of EngineKind
	#[derive(Debug, Error)]
	pub enum EngineKindFromStrError {
		/// The string isn't a recognized EngineKind
		#[error("unknown engine kind {0}")]
		Unknown(String),
	}
}
