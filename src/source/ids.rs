use crate::{
	manifest::target::TargetKind,
	ser_display_deser_fromstr,
	source::{PackageSources, refs::PackageRefs},
};
use semver::Version;
use std::{fmt::Display, str::FromStr};

/// A version ID, which is a combination of a version and a target
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VersionId(Version, TargetKind);
ser_display_deser_fromstr!(VersionId);

impl VersionId {
	/// Creates a new version ID
	#[must_use]
	pub fn new(version: Version, target: TargetKind) -> Self {
		VersionId(version, target)
	}

	/// Access the version
	#[must_use]
	pub fn version(&self) -> &Version {
		&self.0
	}

	/// Access the target
	#[must_use]
	pub fn target(&self) -> TargetKind {
		self.1
	}

	/// Returns this version ID as a string that can be used in the filesystem
	#[must_use]
	pub fn escaped(&self) -> String {
		format!("{}+{}", self.0, self.1)
	}

	/// The reverse of `escaped`
	pub fn from_escaped(s: &str) -> Result<Self, errors::VersionIdParseError> {
		VersionId::from_str(s.replacen('+', " ", 1).as_str())
	}

	/// Access the parts of the version ID
	#[must_use]
	pub fn parts(&self) -> (&Version, TargetKind) {
		(&self.0, self.1)
	}
}

impl Display for VersionId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{} {}", self.0, self.1)
	}
}

impl FromStr for VersionId {
	type Err = errors::VersionIdParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let Some((version, target)) = s.split_once(' ') else {
			return Err(errors::VersionIdParseError::Malformed(s.to_string()));
		};

		let version = version.parse()?;
		let target = target.parse()?;

		Ok(VersionId(version, target))
	}
}

/// A package ID, which is a combination of a name and a version ID
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageId(PackageSources, PackageRefs, VersionId);
ser_display_deser_fromstr!(PackageId);

impl PackageId {
	/// Creates a new package ID
	#[must_use]
	pub fn new(source: PackageSources, pkg_ref: PackageRefs, v_id: VersionId) -> Self {
		PackageId(source, pkg_ref, v_id)
	}

	/// Accesses the package source
	#[must_use]
	pub fn source(&self) -> &PackageSources {
		&self.0
	}

	/// Accesses the package ref
	#[must_use]
	pub fn pkg_ref(&self) -> &PackageRefs {
		&self.1
	}

	/// Accesses the version id
	#[must_use]
	pub fn v_id(&self) -> &VersionId {
		&self.2
	}

	/// Returns a filesystem safe version of this id
	#[must_use]
	pub fn escaped(&self) -> String {
		self.to_string()
			.chars()
			.map(|c| {
				if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '+' {
					c
				} else {
					'-'
				}
			})
			.collect()
	}
}

impl Display for PackageId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let pkg_ref = match self.pkg_ref() {
			PackageRefs::Pesde(pkg_ref) => pkg_ref.to_string(),
			PackageRefs::Wally(pkg_ref) => pkg_ref.to_string(),
			PackageRefs::Git(pkg_ref) => pkg_ref.to_string(),
			PackageRefs::Path(pkg_ref) => pkg_ref.to_string(),
		};

		write!(f, "{}|{pkg_ref}|{}", self.source(), self.v_id())
	}
}

impl FromStr for PackageId {
	type Err = errors::PackageIdParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let mut stream = s.chars();
		let source = stream
			.by_ref()
			.take_while(|c| *c != '|')
			.collect::<String>();
		let source = source.parse()?;

		let pkg_ref = stream
			.by_ref()
			.take_while(|c| *c != '|')
			.collect::<String>();
		let pkg_ref = match source {
			PackageSources::Pesde(_) => pkg_ref.parse().map(PackageRefs::Pesde)?,
			PackageSources::Wally(_) => pkg_ref.parse().map(PackageRefs::Wally)?,
			PackageSources::Git(_) => pkg_ref.parse().map(PackageRefs::Git)?,
			// infallible
			PackageSources::Path(_) => pkg_ref.parse().map(PackageRefs::Path).unwrap(),
		};

		let v_id = stream.collect::<String>().parse()?;

		Ok(PackageId::new(source, pkg_ref, v_id))
	}
}

/// Errors that can occur when using a version ID
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when parsing a version ID
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum VersionIdParseError {
		/// The version ID is malformed
		#[error("malformed version id {0}")]
		Malformed(String),

		/// The version is malformed
		#[error("malformed version")]
		Version(#[from] semver::Error),

		/// The target is malformed
		#[error("malformed target")]
		Target(#[from] crate::manifest::target::errors::TargetKindFromStr),
	}

	/// Errors that can occur when parsing a package ID
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum PackageIdParseError {
		/// Parsing the source failed
		#[error("error parsing package source")]
		PackageSource(#[from] crate::source::errors::PackageSourcesFromStr),

		/// Parsing the pesde package reference failed
		#[error("error parsing package reference")]
		PesdePackageRef(#[from] crate::names::errors::PackageNameError),

		/// Parsing the Wally package reference failed
		#[cfg(feature = "wally-compat")]
		#[error("error parsing wally package reference")]
		WallyPackageRef(#[from] crate::names::errors::WallyPackageNameError),

		/// Parsing the Git package reference failed
		#[error("error parsing git package reference")]
		PackageRef(#[from] crate::source::refs::errors::GitPackageRefParseError),

		/// Parsing the VersionId failed
		#[error("error parsing version id")]
		VersionId(#[from] VersionIdParseError),
	}
}
