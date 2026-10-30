//! Package names
use pesde_core::registry::{LocalName, Scope};

// TODO: remove this module and put the structs in their appropriate source modules
use crate::ser_display_deser_fromstr;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

/// A pesde package name
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageName(Arc<(Scope, LocalName)>);

ser_display_deser_fromstr!(PackageName);

impl FromStr for PackageName {
	type Err = errors::PackageNameError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (scope, local_name) = s
			.split_once('/')
			.ok_or_else(|| errors::PackageNameErrorKind::InvalidFormat(s.to_string()))?;

		Ok(Self(Arc::new((
			Scope::from_str(scope)?,
			LocalName::from_str(local_name)?,
		))))
	}
}

impl Display for PackageName {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}/{}", self.scope(), self.local_name())
	}
}

impl PackageName {
	/// Creates a new `PackageName` from already-validated parts
	#[must_use]
	pub fn new(scope: Scope, local_name: LocalName) -> Self {
		Self(Arc::new((scope, local_name)))
	}

	/// Returns the scope of the package name
	#[must_use]
	pub fn scope(&self) -> &Scope {
		&self.0.0
	}

	/// Returns the name part of the package name
	#[must_use]
	pub fn local_name(&self) -> &LocalName {
		&self.0.1
	}
}

/// Which part of a package name is invalid
#[derive(Debug)]
pub enum ErrorPart {
	/// The scope of the package name is invalid
	Scope,
	/// The local name of the package name is invalid
	LocalName,
}

impl Display for ErrorPart {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			ErrorPart::Scope => write!(f, "scope"),
			ErrorPart::LocalName => write!(f, "local name"),
		}
	}
}

/// A Wally package name
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WallyPackageName(Arc<(Box<str>, Box<str>)>);
ser_display_deser_fromstr!(WallyPackageName);

impl FromStr for WallyPackageName {
	type Err = errors::WallyPackageNameError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (scope, local_name) = s
			.split_once('/')
			.ok_or_else(|| errors::WallyPackageNameErrorKind::InvalidFormat(s.to_string()))?;

		for (reason, part) in [
			(ErrorPart::Scope, scope),
			(ErrorPart::LocalName, local_name),
		] {
			if part.is_empty() || part.len() > 64 {
				return Err(errors::WallyPackageNameErrorKind::InvalidLength(
					reason,
					part.to_string(),
				)
				.into());
			}

			if !part
				.chars()
				.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
			{
				return Err(errors::WallyPackageNameErrorKind::InvalidCharacters(
					reason,
					part.to_string(),
				)
				.into());
			}
		}

		Ok(Self(Arc::new((scope.into(), local_name.into()))))
	}
}

impl Display for WallyPackageName {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}/{}", self.scope(), self.local_name())
	}
}

impl WallyPackageName {
	/// Returns the scope of the package name
	#[must_use]
	pub fn scope(&self) -> &str {
		&self.0.0
	}

	/// Returns the local name of the package name
	#[must_use]
	pub fn local_name(&self) -> &str {
		&self.0.1
	}
}

/// Errors that can occur when working with package names
pub mod errors {
	use thiserror::Error;

	use crate::names::ErrorPart;

	/// Errors that can occur when working with pesde package names
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = PackageNameError))]
	pub enum PackageNameErrorKind {
		/// The package name is not in the format `scope/local_name`
		#[error("package name `{0}` is not in the format `scope/local_name`")]
		InvalidFormat(String),

		/// The scope name is invalid
		#[error("invalid scope name")]
		ScopeName(#[from] pesde_core::registry::ScopeFromStrError),

		/// The local name is invalid
		#[error("invalid local name")]
		LocalNameName(#[from] pesde_core::registry::LocalNameFromStrError),
	}

	/// Errors that can occur when working with Wally package names
	#[expect(clippy::enum_variant_names)]
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = WallyPackageNameError))]
	pub enum WallyPackageNameErrorKind {
		/// The package name is not in the format `scope/local_name`
		#[error("wally package name `{0}` is not in the format `scope/local_name`")]
		InvalidFormat(String),

		/// The package name is outside the allowed characters: a-z, 0-9, and -
		#[error("wally package {0} `{1}` contains characters outside a-z, 0-9, and -")]
		InvalidCharacters(ErrorPart, String),

		/// The package name is not within 1-64 characters long
		#[error("wally package {0} `{1}` is not within 1-64 characters long")]
		InvalidLength(ErrorPart, String),
	}
}
