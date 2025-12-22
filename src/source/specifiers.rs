#![expect(deprecated)]
use crate::source::{pesde, traits::DependencySpecifier};
use serde::{Deserialize, Serialize};
use std::fmt::Display;

/// All possible dependency specifiers
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum DependencySpecifiers {
	/// A pesde dependency specifier
	Pesde(pesde::specifier::PesdeDependencySpecifier),
	/// A Wally dependency specifier
	#[cfg(feature = "wally-compat")]
	Wally(crate::source::wally::specifier::WallyDependencySpecifier),
	/// A Git dependency specifier
	Git(crate::source::git::specifier::GitDependencySpecifier),
	/// A path dependency specifier
	Path(crate::source::path::specifier::PathDependencySpecifier),
}

impl DependencySpecifiers {
	/// Returns whether this dependency specifier is for a local dependency
	#[must_use]
	pub fn is_local(&self) -> bool {
		matches!(self, DependencySpecifiers::Path(_))
	}
}

impl DependencySpecifier for DependencySpecifiers {}

impl Display for DependencySpecifiers {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			DependencySpecifiers::Pesde(specifier) => write!(f, "{specifier}"),
			#[cfg(feature = "wally-compat")]
			DependencySpecifiers::Wally(specifier) => write!(f, "{specifier}"),
			DependencySpecifiers::Git(specifier) => write!(f, "{specifier}"),
			DependencySpecifiers::Path(specifier) => write!(f, "{specifier}"),
		}
	}
}
