use thiserror::Error;

use crate::__private::{is_reserved_file_name, ser_display_deser_fromstr};
use std::{fmt::Display, hash::Hash, ops::Deref, str::FromStr, sync::Arc};

/// An alias of a dependency
/// Equality checks (Ord, PartialOrd, PartialEq, Eq, Hash) are case-insensitive
#[derive(Debug, Clone)]
pub struct Alias(Arc<str>);
ser_display_deser_fromstr!(Alias);

impl Ord for Alias {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.0.to_lowercase().cmp(&other.0.to_lowercase())
	}
}

impl PartialOrd for Alias {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl PartialEq for Alias {
	fn eq(&self, other: &Self) -> bool {
		self.0.to_lowercase() == other.0.to_lowercase()
	}
}

impl Eq for Alias {}

impl Hash for Alias {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.0.to_lowercase().hash(state);
	}
}

impl Display for Alias {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

impl FromStr for Alias {
	type Err = AliasFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if s.is_empty() {
			return Err(Self::Err::Empty);
		}

		if s.len() > 48 {
			return Err(Self::Err::TooLong(s.into()));
		}

		let pesde_reserved = [
			// Luau's `@self` alias
			"self", // Luau's `init.luau` file
			"init", "pesde",
		];
		if is_reserved_file_name(s)
			|| pesde_reserved
				.iter()
				.any(|&reserved| reserved.eq_ignore_ascii_case(s))
		{
			return Err(Self::Err::Reserved(s.into()));
		}

		if !s
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
		{
			return Err(Self::Err::InvalidCharacters(s.into()));
		}

		Ok(Self(s.into()))
	}
}

impl Alias {
	/// Get the alias as a string
	#[must_use]
	pub fn as_str(&self) -> &str {
		self
	}
}

impl Deref for Alias {
	type Target = str;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

/// Errors that can occur when parsing an alias from a string
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AliasFromStrError {
	/// The alias is empty
	#[error("the alias is empty")]
	Empty,

	/// The alias has more than 48 characters
	#[error("alias `{0}` has more than 48 characters")]
	TooLong(Box<str>),

	/// The alias is reserved
	#[error("alias `{0}` is reserved")]
	Reserved(Box<str>),

	/// The alias contains characters outside a-z, A-Z, 0-9, -, and _
	#[error("alias `{0}` contains characters outside a-z, A-Z, 0-9, -, and _")]
	InvalidCharacters(Box<str>),
}
