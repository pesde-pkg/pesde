use crate::{manifest::target::TargetKind, names::PackageNames, ser_display_deser_fromstr};
use semver::Version;
use std::{fmt::Display, str::FromStr};

/// A version ID, which is a combination of a version and a target
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VersionId(pub(crate) Version, pub(crate) TargetKind);
ser_display_deser_fromstr!(VersionId);

impl VersionId {
	/// Creates a new version ID
	pub fn new(version: Version, target: TargetKind) -> Self {
		VersionId(version, target)
	}

	/// Access the version
	pub fn version(&self) -> &Version {
		&self.0
	}

	/// Access the target
	pub fn target(&self) -> TargetKind {
		self.1
	}

	/// Returns this version ID as a string that can be used in the filesystem
	pub fn escaped(&self) -> String {
		format!("{}+{}", self.0, self.1)
	}

	/// The reverse of `escaped`
	pub fn from_escaped(s: &str) -> Result<Self, errors::VersionIdParseError> {
		VersionId::from_str(s.replacen('+', " ", 1).as_str())
	}

	/// Access the parts of the version ID
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

#[cfg(feature = "schema")]
impl schemars::JsonSchema for VersionId {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"VersionId".into()
	}

	fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
		let version_schema = Version::json_schema(&mut schemars::SchemaGenerator::default());
		let version_pattern = version_schema
			.get("pattern")
			.unwrap()
			.as_str()
			.unwrap()
			.trim_start_matches('^')
			.trim_end_matches('$');

		let target_pattern = TargetKind::VARIANTS
			.iter()
			.map(ToString::to_string)
			.collect::<Vec<_>>()
			.join("|");

		schemars::json_schema!({
			"type": "string",
			"pattern": format!(r#"^({version_pattern}) ({target_pattern})$"#),
		})
	}
}

/// A package ID, which is a combination of a name and a version ID
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageId(pub(crate) PackageNames, pub(crate) VersionId);
ser_display_deser_fromstr!(PackageId);

impl PackageId {
	/// Creates a new package ID
	pub fn new(names: PackageNames, version_id: VersionId) -> Self {
		PackageId(names, version_id)
	}

	/// Access the name
	pub fn name(&self) -> &PackageNames {
		&self.0
	}

	/// Access the version ID
	pub fn version_id(&self) -> &VersionId {
		&self.1
	}

	/// Access the parts of the package ID
	pub fn parts(&self) -> (&PackageNames, &VersionId) {
		(&self.0, &self.1)
	}
}

impl Display for PackageId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}@{}", self.0, self.1)
	}
}

impl FromStr for PackageId {
	type Err = errors::PackageIdParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let Some((names, version_id)) = s.split_once('@') else {
			return Err(errors::PackageIdParseError::Malformed(s.to_string()));
		};

		let names = names.parse()?;
		let version_id = version_id.parse()?;

		Ok(PackageId(names, version_id))
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
		/// The package ID is malformed (not in the form `name@version`)
		#[error("malformed package id {0}")]
		Malformed(String),

		/// The name is malformed
		#[error("malformed name")]
		Names(#[from] crate::names::errors::PackageNamesError),

		/// The version ID is malformed
		#[error("malformed version id")]
		VersionId(#[from] VersionIdParseError),
	}
}
