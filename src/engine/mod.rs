/// Engines as runtimes
pub mod runtime;
/// Sources of engines
pub mod source;

use crate::{engine::source::EngineSources, ser_display_deser_fromstr};
use std::{fmt::Display, str::FromStr};

/// All supported engines
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EngineKind {
	/// The pesde package manager
	Pesde,
	/// The Lune runtime
	Lune,
}
ser_display_deser_fromstr!(EngineKind);

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
	/// All [EngineKind]s
	pub const VARIANTS: &'static [EngineKind] = &[EngineKind::Pesde, EngineKind::Lune];

	/// Returns the source to get this engine from
	#[must_use]
	pub fn source(self) -> EngineSources {
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
