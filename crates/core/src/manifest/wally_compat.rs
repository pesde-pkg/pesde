use std::{fmt::Display, ops::Deref, str::FromStr, sync::Arc};

use crate::__private::ser_display_deser_fromstr;
use paste::paste;
use thiserror::Error;

macro_rules! name_part {
	($name:ident) => {
		paste! {
			#[doc = concat!("A validated package ", stringify!([< $name:lower ]>), "(", stringify!($min), "-64 chars, a-z/0-9/_, no leading/trailing _, not all digits)")]
			#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
			pub struct $name(Arc<str>);
			ser_display_deser_fromstr!($name);

			impl $name {
				pub fn as_str(&self) -> &str {
					self
				}
			}

			impl Display for $name {
				fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
					f.write_str(self)
				}
			}

			impl FromStr for $name {
				type Err = [< $name FromStrError >];

				fn from_str(s: &str) -> Result<Self, Self::Err> {
					if !(1..=64).contains(&s.len()) {
						return Err(Self::Err::InvalidLength(s.into()));
					}

					if !s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
						return Err(Self::Err::InvalidCharacters(s.into()));
					}

					Ok(Self(s.into()))
				}
			}

			#[doc = concat!("Errors that can occur when parsing a [", stringify!($name), "] from str")]
			#[derive(Debug, Error)]
			pub enum [< $name FromStrError >] {
				/// The name is outside the allowed characters: a-z, 0-9, and -
				#[error("name `{0}` contains characters outside a-z, 0-9, and -")]
				InvalidCharacters(Box<str>),

				/// The name's scope part is not within 1-64 characters long
				#[error("name `{0}` is not within 1-64 characters long")]
				InvalidLength(Box<str>),
			}

			impl Deref for $name {
				type Target = str;

				fn deref(&self) -> &Self::Target {
					&self.0
				}
			}
		}
	};
}

name_part!(WallyScope);
name_part!(WallyLocalName);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WallyPackageName(Arc<(WallyScope, WallyLocalName)>);
ser_display_deser_fromstr!(WallyPackageName);

impl WallyPackageName {
	/// Creates a new [Self] from a [WallyScope] and a [WallyLocalName]
	#[must_use]
	pub fn new(scope: WallyScope, local_name: WallyLocalName) -> Self {
		Self(Arc::new((scope, local_name)))
	}

	/// The scope of the package
	#[must_use]
	pub fn scope(&self) -> &WallyScope {
		&self.0.0
	}

	/// The local name of the package
	#[must_use]
	pub fn local_name(&self) -> &WallyLocalName {
		&self.0.1
	}
}

impl Display for WallyPackageName {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}/{}", self.scope(), self.local_name())
	}
}

impl FromStr for WallyPackageName {
	type Err = WallyPackageNameFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (scope, local_name) = s
			.split_once('/')
			.ok_or_else(|| Self::Err::InvalidFormat(s.into()))?;

		Ok(Self::new(
			scope.parse().map_err(Self::Err::ScopeParse)?,
			local_name.parse().map_err(Self::Err::LocalNameParse)?,
		))
	}
}

/// Errors that can occur when parsing a [WallyPackageName] from str
#[derive(Debug, Error)]
pub enum WallyPackageNameFromStrError {
	/// The value wasn't in format `scope/local_name`
	#[error("value `{0}` wasn't in format of `scope/local_name`")]
	InvalidFormat(Box<str>),

	/// An error occurred while parsing the scope
	#[error("error parsing scope")]
	ScopeParse(#[from] WallyScopeFromStrError),

	/// An error occurred while parsing the local name
	#[error("error parsing local name")]
	LocalNameParse(#[from] WallyLocalNameFromStrError),
}
