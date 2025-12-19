use crate::{manifest::Alias, ser_display_deser_fromstr, source::specifiers::DependencySpecifiers};
use serde::{Deserialize, Serialize};
use std::{
	fmt::{Display, Formatter},
	str::FromStr,
};

/// An override key
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OverrideKey(pub Vec<Vec<Alias>>);
ser_display_deser_fromstr!(OverrideKey);

impl FromStr for OverrideKey {
	type Err = errors::OverrideKeyFromStr;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let overrides = s
			.split(',')
			.map(|overrides| {
				overrides
					.split('>')
					.map(Alias::from_str)
					.collect::<Result<_, _>>()
			})
			.collect::<Result<Vec<Vec<Alias>>, _>>()?;

		if overrides.is_empty() {
			return Err(errors::OverrideKeyFromStr::Empty);
		}

		Ok(Self(overrides))
	}
}

impl Display for OverrideKey {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"{}",
			self.0
				.iter()
				.map(|overrides| {
					overrides
						.iter()
						.map(Alias::as_str)
						.collect::<Vec<_>>()
						.join(">")
				})
				.collect::<Vec<_>>()
				.join(",")
		)
	}
}

/// A specifier for an override
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum OverrideSpecifier {
	/// A specifier for a dependency
	Specifier(DependencySpecifiers),
	/// An alias for a dependency the current project depends on
	Alias(Alias),
}

/// Errors that can occur when interacting with override keys
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when parsing an override key
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum OverrideKeyFromStr {
		/// The override key is empty
		#[error("empty override key")]
		Empty,

		/// An alias in the override key is invalid
		#[error("invalid alias in override key")]
		InvalidAlias(#[from] crate::manifest::errors::AliasFromStr),
	}
}
