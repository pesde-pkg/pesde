#![expect(deprecated)]
use crate::ser_display_deser_fromstr;
use std::{fmt::Display, str::FromStr, sync::Arc};

/// The invalid part of a package name
#[derive(Debug)]
pub enum ErrorReason {
	/// The scope of the package name is invalid
	Scope,
	/// The name of the package name is invalid
	Name,
}

impl Display for ErrorReason {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			ErrorReason::Scope => write!(f, "scope"),
			ErrorReason::Name => write!(f, "name"),
		}
	}
}

/// A pesde package name
#[deprecated = "pesde has dropped registries. See https://github.com/pesde-pkg/pesde/issues/59"]
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageName(Arc<(Box<str>, Box<str>)>);

ser_display_deser_fromstr!(PackageName);

impl FromStr for PackageName {
	type Err = errors::PackageNameError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (scope, name) = s
			.split_once('/')
			.ok_or_else(|| Self::Err::InvalidFormat(s.to_string()))?;

		for (reason, part) in [(ErrorReason::Scope, scope), (ErrorReason::Name, name)] {
			let min_len = match reason {
				ErrorReason::Scope => 3,
				ErrorReason::Name => 1,
			};

			if !(min_len..=32).contains(&part.len()) {
				return Err(match reason {
					ErrorReason::Scope => Self::Err::InvalidScopeLength(part.to_string()),
					ErrorReason::Name => Self::Err::InvalidNameLength(part.to_string()),
				});
			}

			if part.chars().all(|c| c.is_ascii_digit()) {
				return Err(Self::Err::OnlyDigits(reason, part.to_string()));
			}

			if part.starts_with('_') || part.ends_with('_') {
				return Err(Self::Err::PrePostfixUnderscore(reason, part.to_string()));
			}

			if !part
				.chars()
				.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
			{
				return Err(Self::Err::InvalidCharacters(reason, part.to_string()));
			}
		}

		Ok(Self(Arc::new((scope.into(), name.into()))))
	}
}

impl Display for PackageName {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}/{}", self.scope(), self.name())
	}
}

impl PackageName {
	/// Returns the parts of the package name
	#[must_use]
	pub fn as_str(&self) -> (&str, &str) {
		(self.scope(), self.name())
	}

	/// Returns the package name as a string suitable for use in the filesystem
	#[must_use]
	pub fn escaped(&self) -> String {
		format!("{}+{}", self.scope(), self.name())
	}

	/// Returns the scope of the package name
	#[must_use]
	pub fn scope(&self) -> &str {
		&self.0.0
	}

	/// Returns the name of the package name
	#[must_use]
	pub fn name(&self) -> &str {
		&self.0.1
	}
}

/// All possible package names
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum PackageNames {
	/// A pesde package name
	Pesde(PackageName),
	/// A Wally package name
	#[cfg(feature = "wally-compat")]
	Wally(wally::WallyPackageName),
}
ser_display_deser_fromstr!(PackageNames);

impl PackageNames {
	/// Returns the parts of the package name
	#[must_use]
	pub fn as_str(&self) -> (&str, &str) {
		match self {
			PackageNames::Pesde(name) => name.as_str(),
			#[cfg(feature = "wally-compat")]
			PackageNames::Wally(name) => name.as_str(),
		}
	}

	/// Returns the package name as a string suitable for use in the filesystem
	#[must_use]
	pub fn escaped(&self) -> String {
		match self {
			PackageNames::Pesde(name) => name.escaped(),
			#[cfg(feature = "wally-compat")]
			PackageNames::Wally(name) => name.escaped(),
		}
	}

	/// Returns the scope of the package name
	#[must_use]
	pub fn scope(&self) -> &str {
		match self {
			PackageNames::Pesde(name) => name.scope(),
			#[cfg(feature = "wally-compat")]
			PackageNames::Wally(name) => name.scope(),
		}
	}

	/// Returns the name of the package name
	#[must_use]
	pub fn name(&self) -> &str {
		match self {
			PackageNames::Pesde(name) => name.name(),
			#[cfg(feature = "wally-compat")]
			PackageNames::Wally(name) => name.name(),
		}
	}
}

impl Display for PackageNames {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			PackageNames::Pesde(name) => write!(f, "{name}"),
			#[cfg(feature = "wally-compat")]
			PackageNames::Wally(name) => write!(f, "wally#{name}"),
		}
	}
}

impl FromStr for PackageNames {
	type Err = errors::PackageNamesError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		#[cfg(feature = "wally-compat")]
		if let Some(wally_name) = s
			.strip_prefix("wally#")
			.or_else(|| s.contains('-').then_some(s))
			.and_then(|s| wally::WallyPackageName::from_str(s).ok())
		{
			return Ok(PackageNames::Wally(wally_name));
		}

		if let Ok(name) = PackageName::from_str(s) {
			Ok(PackageNames::Pesde(name))
		} else {
			Err(errors::PackageNamesError::InvalidPackageName(s.to_string()))
		}
	}
}

/// Wally package names
#[cfg(feature = "wally-compat")]
pub mod wally {
	use std::{fmt::Display, str::FromStr, sync::Arc};

	use crate::{
		names::{ErrorReason, errors},
		ser_display_deser_fromstr,
	};

	/// A Wally package name
	#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
	pub struct WallyPackageName(Arc<(Box<str>, Box<str>)>);
	ser_display_deser_fromstr!(WallyPackageName);

	impl FromStr for WallyPackageName {
		type Err = errors::WallyPackageNameError;

		fn from_str(s: &str) -> Result<Self, Self::Err> {
			// backwards compatibility
			let s = s.strip_prefix("wally#").unwrap_or(s);

			let (scope, name) = s
				.split_once('/')
				.ok_or_else(|| Self::Err::InvalidFormat(s.to_string()))?;

			for (reason, part) in [(ErrorReason::Scope, scope), (ErrorReason::Name, name)] {
				if part.is_empty() || part.len() > 64 {
					return Err(Self::Err::InvalidLength(reason, part.to_string()));
				}

				if !part
					.chars()
					.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
				{
					return Err(Self::Err::InvalidCharacters(reason, part.to_string()));
				}
			}

			Ok(Self(Arc::new((scope.into(), name.into()))))
		}
	}

	impl Display for WallyPackageName {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			write!(f, "{}/{}", self.scope(), self.name())
		}
	}

	impl WallyPackageName {
		/// Returns the parts of the package name
		#[must_use]
		pub fn as_str(&self) -> (&str, &str) {
			(self.scope(), self.name())
		}

		/// Returns the package name as a string suitable for use in the filesystem
		#[must_use]
		pub fn escaped(&self) -> String {
			format!("{}+{}", self.scope(), self.name())
		}

		/// Returns the scope of the package name
		#[must_use]
		pub fn scope(&self) -> &str {
			&self.0.0
		}

		/// Returns the name of the package name
		#[must_use]
		pub fn name(&self) -> &str {
			&self.0.1
		}
	}
}

/// Errors that can occur when working with package names
pub mod errors {
	use thiserror::Error;

	use crate::names::ErrorReason;

	/// Errors that can occur when working with pesde package names
	#[derive(Debug, Error)]
	pub enum PackageNameError {
		/// The package name is not in the format `scope/name`
		#[error("package name `{0}` is not in the format `scope/name`")]
		InvalidFormat(String),

		/// The package name is outside the allowed characters: a-z, 0-9, and _
		#[error("package {0} `{1}` contains characters outside a-z, 0-9, and _")]
		InvalidCharacters(ErrorReason, String),

		/// The package name contains only digits
		#[error("package {0} `{1}` contains only digits")]
		OnlyDigits(ErrorReason, String),

		/// The package name starts or ends with an underscore
		#[error("package {0} `{1}` starts or ends with an underscore")]
		PrePostfixUnderscore(ErrorReason, String),

		/// The package name's scope part is not within 3-32 characters long
		#[error("package scope `{0}` is not within 3-32 characters long")]
		InvalidScopeLength(String),

		/// The package name's name part is not within 1-32 characters long
		#[error("package name `{0}` is not within 1-32 characters long")]
		InvalidNameLength(String),
	}

	/// Errors that can occur when working with Wally package names
	#[cfg(feature = "wally-compat")]
	#[allow(clippy::enum_variant_names)]
	#[derive(Debug, Error)]
	pub enum WallyPackageNameError {
		/// The package name is not in the format `scope/name`
		#[error("wally package name `{0}` is not in the format `scope/name`")]
		InvalidFormat(String),

		/// The package name is outside the allowed characters: a-z, 0-9, and -
		#[error("wally package {0} `{1}` contains characters outside a-z, 0-9, and -")]
		InvalidCharacters(ErrorReason, String),

		/// The package name is not within 1-64 characters long
		#[error("wally package {0} `{1}` is not within 1-64 characters long")]
		InvalidLength(ErrorReason, String),
	}

	/// Errors that can occur when working with package names
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum PackageNamesError {
		/// The package name is invalid
		#[error("invalid package name {0}")]
		InvalidPackageName(String),
	}
}
