//! Package names
// TODO: remove this module and put the structs in their appropriate source modules
use crate::ser_display_deser_fromstr;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

/// A validated package scope (3–32 chars, a-z/0-9/_, no leading/trailing _, not all digits)
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Scope(Arc<str>);

ser_display_deser_fromstr!(Scope);

impl FromStr for Scope {
	type Err = errors::PackageNameError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if !(3..=32).contains(&s.len()) {
			return Err(errors::PackageNameErrorKind::InvalidScopeLength(s.to_string()).into());
		}
		if s.chars().all(|c| c.is_ascii_digit()) {
			return Err(
				errors::PackageNameErrorKind::OnlyDigits(ErrorPart::Scope, s.to_string()).into(),
			);
		}
		if s.starts_with('_') || s.ends_with('_') {
			return Err(errors::PackageNameErrorKind::PrePostfixUnderscore(
				ErrorPart::Scope,
				s.to_string(),
			)
			.into());
		}
		if !s
			.chars()
			.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
		{
			return Err(errors::PackageNameErrorKind::InvalidCharacters(
				ErrorPart::Scope,
				s.to_string(),
			)
			.into());
		}
		Ok(Self(s.into()))
	}
}

impl Display for Scope {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

impl Scope {
	/// Returns the scope as a str
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

/// A validated package name part (1–32 chars, a-z/0-9/_, no leading/trailing _, not all digits)
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(Arc<str>);

ser_display_deser_fromstr!(Name);

impl FromStr for Name {
	type Err = errors::PackageNameError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if !(1..=32).contains(&s.len()) {
			return Err(errors::PackageNameErrorKind::InvalidNameLength(s.to_string()).into());
		}
		if s.chars().all(|c| c.is_ascii_digit()) {
			return Err(
				errors::PackageNameErrorKind::OnlyDigits(ErrorPart::Name, s.to_string()).into(),
			);
		}
		if s.starts_with('_') || s.ends_with('_') {
			return Err(errors::PackageNameErrorKind::PrePostfixUnderscore(
				ErrorPart::Name,
				s.to_string(),
			)
			.into());
		}
		if !s
			.chars()
			.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
		{
			return Err(errors::PackageNameErrorKind::InvalidCharacters(
				ErrorPart::Name,
				s.to_string(),
			)
			.into());
		}
		Ok(Self(s.into()))
	}
}

impl Display for Name {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

impl Name {
	/// Returns the name as a str
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

/// Which part of a package name is invalid
#[derive(Debug)]
pub enum ErrorPart {
	/// The scope of the package name is invalid
	Scope,
	/// The name of the package name is invalid
	Name,
}

impl Display for ErrorPart {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			ErrorPart::Scope => write!(f, "scope"),
			ErrorPart::Name => write!(f, "name"),
		}
	}
}

/// A pesde package name
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageName(Arc<(Scope, Name)>);

ser_display_deser_fromstr!(PackageName);

impl FromStr for PackageName {
	type Err = errors::PackageNameError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (scope, name) = s
			.split_once('/')
			.ok_or_else(|| errors::PackageNameErrorKind::InvalidFormat(s.to_string()))?;

		Ok(Self(Arc::new((
			Scope::from_str(scope)?,
			Name::from_str(name)?,
		))))
	}
}

impl Display for PackageName {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}/{}", self.scope(), self.name())
	}
}

impl PackageName {
	/// Creates a new `PackageName` from already-validated parts
	#[must_use]
	pub fn new(scope: Scope, name: Name) -> Self {
		Self(Arc::new((scope, name)))
	}

	/// Returns the scope of the package name
	#[must_use]
	pub fn scope(&self) -> &Scope {
		&self.0.0
	}

	/// Returns the name part of the package name
	#[must_use]
	pub fn name(&self) -> &Name {
		&self.0.1
	}
}

/// A Wally package name
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WallyPackageName(Arc<(Box<str>, Box<str>)>);
ser_display_deser_fromstr!(WallyPackageName);

impl FromStr for WallyPackageName {
	type Err = errors::WallyPackageNameError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (scope, name) = s
			.split_once('/')
			.ok_or_else(|| errors::WallyPackageNameErrorKind::InvalidFormat(s.to_string()))?;

		for (reason, part) in [(ErrorPart::Scope, scope), (ErrorPart::Name, name)] {
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

		Ok(Self(Arc::new((scope.into(), name.into()))))
	}
}

impl Display for WallyPackageName {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}/{}", self.scope(), self.name())
	}
}

impl WallyPackageName {
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

/// Errors that can occur when working with package names
pub mod errors {
	use thiserror::Error;

	use crate::names::ErrorPart;

	/// Errors that can occur when working with pesde package names
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = PackageNameError))]
	pub enum PackageNameErrorKind {
		/// The package name is not in the format `scope/name`
		#[error("package name `{0}` is not in the format `scope/name`")]
		InvalidFormat(String),

		/// The package name is outside the allowed characters: a-z, 0-9, and _
		#[error("package {0} `{1}` contains characters outside a-z, 0-9, and _")]
		InvalidCharacters(ErrorPart, String),

		/// The package name contains only digits
		#[error("package {0} `{1}` contains only digits")]
		OnlyDigits(ErrorPart, String),

		/// The package name starts or ends with an underscore
		#[error("package {0} `{1}` starts or ends with an underscore")]
		PrePostfixUnderscore(ErrorPart, String),

		/// The package name's scope part is not within 3-32 characters long
		#[error("package scope `{0}` is not within 3-32 characters long")]
		InvalidScopeLength(String),

		/// The package name's name part is not within 1-32 characters long
		#[error("package name `{0}` is not within 1-32 characters long")]
		InvalidNameLength(String),
	}

	/// Errors that can occur when working with Wally package names
	#[expect(clippy::enum_variant_names)]
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = WallyPackageNameError))]
	pub enum WallyPackageNameErrorKind {
		/// The package name is not in the format `scope/name`
		#[error("wally package name `{0}` is not in the format `scope/name`")]
		InvalidFormat(String),

		/// The package name is outside the allowed characters: a-z, 0-9, and -
		#[error("wally package {0} `{1}` contains characters outside a-z, 0-9, and -")]
		InvalidCharacters(ErrorPart, String),

		/// The package name is not within 1-64 characters long
		#[error("wally package {0} `{1}` is not within 1-64 characters long")]
		InvalidLength(ErrorPart, String),
	}

	/// Errors that can occur when working with package names
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = PackageNamesError))]
	#[non_exhaustive]
	pub enum PackageNamesErrorKind {
		/// The package name is invalid
		#[error("invalid package name {0}")]
		InvalidPackageName(String),
	}
}
