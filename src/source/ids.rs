use crate::manifest::target::TargetKind;
use crate::ser_display_deser_fromstr;
use crate::source::PackageSources;
use crate::source::errors::PackageSourcesFromStr;
use crate::source::path::PathPackageSource;
use crate::source::path::local_version;
use crate::source::refs::PackageRefs;
use crate::source::refs::errors::PackageRefParseError;
use semver::Version;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

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
		format!("{}+{}", self.version(), self.target())
	}

	/// Access the parts of the version ID
	#[must_use]
	pub fn parts(&self) -> (&Version, TargetKind) {
		(self.version(), self.target())
	}
}

impl Display for VersionId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}:{}", self.version(), self.target())
	}
}

impl FromStr for VersionId {
	type Err = errors::VersionIdParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (version, target) = s
			.split_once([':', ' '])
			.ok_or(errors::VersionIdParseErrorKind::Malformed(s.to_string()))?;

		let version = version.parse()?;
		let target = target.parse()?;

		Ok(VersionId(version, target))
	}
}

/// A package ID, which is a combination of a name and a version ID
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageId(Arc<(PackageSources, PackageRefs, VersionId)>);
ser_display_deser_fromstr!(PackageId);

impl PackageId {
	/// Creates a new package ID
	#[must_use]
	pub fn new(source: PackageSources, pkg_ref: PackageRefs, v_id: VersionId) -> Self {
		PackageId(Arc::new((source, pkg_ref, v_id)))
	}

	/// Accesses the package source
	#[must_use]
	pub fn source(&self) -> &PackageSources {
		&self.0.0
	}

	/// Accesses the package ref
	#[must_use]
	pub fn pkg_ref(&self) -> &PackageRefs {
		&self.0.1
	}

	/// Accesses the version id
	#[must_use]
	pub fn v_id(&self) -> &VersionId {
		&self.0.2
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
		let pkg_ref: &dyn Display = match self.pkg_ref() {
			PackageRefs::Pesde(pkg_ref) => pkg_ref,
			PackageRefs::Wally(pkg_ref) => pkg_ref,
			PackageRefs::Git(pkg_ref) => pkg_ref,
			PackageRefs::Path(pkg_ref) => pkg_ref,
		};

		if let PackageSources::Path(_) = self.source() {
			write!(f, "{}:{pkg_ref}:{}", self.source(), self.v_id().target())
		} else {
			let v_id_sep = match self.source() {
				PackageSources::Git(_) => '#',
				_ => '@',
			};

			write!(f, "{}:{pkg_ref}{v_id_sep}{}", self.source(), self.v_id())
		}
	}
}

impl FromStr for PackageId {
	type Err = errors::PackageIdParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (tag, s) = s
			.split_once(':')
			.ok_or(errors::PackageIdParseErrorKind::InvalidFormat)?;

		let v_id_sep = match tag {
			"git" => '#',
			"path" => ':',
			_ => '@',
		};
		let (s, v_id) = s
			.rsplit_once(v_id_sep)
			.ok_or(errors::PackageIdParseErrorKind::InvalidFormat)?;

		let (source, pkg_ref) = if tag == "path" {
			("", s)
		} else {
			s.rsplit_once(':')
				.ok_or(errors::PackageIdParseErrorKind::InvalidFormat)?
		};

		let v_id = if tag == "path" {
			VersionId(
				local_version(),
				v_id.parse().map_err(|e| {
					errors::VersionIdParseError::from(errors::VersionIdParseErrorKind::Target(e))
				})?,
			)
		} else {
			v_id.parse()?
		};

		let source = match tag {
			"pesde" => PackageSources::Pesde(source.parse().map_err(PackageSourcesFromStr::from)?),
			"wally" => PackageSources::Wally(source.parse().map_err(PackageSourcesFromStr::from)?),
			"git" => PackageSources::Git(source.parse().map_err(PackageSourcesFromStr::from)?),
			"path" => PackageSources::Path(PathPackageSource),
			_ => return Err(errors::PackageIdParseErrorKind::InvalidFormat.into()),
		};

		let pkg_ref = match tag {
			"pesde" => PackageRefs::Pesde(pkg_ref.parse().map_err(PackageRefParseError::from)?),
			"wally" => PackageRefs::Wally(pkg_ref.parse().map_err(PackageRefParseError::from)?),
			"git" => PackageRefs::Git(pkg_ref.parse().map_err(PackageRefParseError::from)?),
			// infallible
			"path" => PackageRefs::Path(pkg_ref.parse().unwrap()),
			_ => return Err(errors::PackageIdParseErrorKind::InvalidFormat.into()),
		};

		Ok(PackageId::new(source, pkg_ref, v_id))
	}
}

/// Errors that can occur when using a version ID
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when parsing a version ID
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = VersionIdParseError))]
	#[non_exhaustive]
	pub enum VersionIdParseErrorKind {
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
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = PackageIdParseError))]
	#[non_exhaustive]
	pub enum PackageIdParseErrorKind {
		/// The format of the package ID is invalid
		#[error("invalid package id format")]
		InvalidFormat,

		/// Parsing the source failed
		#[error("error parsing package source")]
		PackageSource(#[from] crate::source::errors::PackageSourcesFromStr),

		/// Parsing the Git package reference failed
		#[error("error parsing git package reference")]
		PackageRef(#[from] crate::source::refs::errors::PackageRefParseError),

		/// Parsing the VersionId failed
		#[error("error parsing version id")]
		VersionId(#[from] VersionIdParseError),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn serde_package_ids() {
		let ids = [
			"pesde:github.com/pesde-pkg/index:foo/bar@1.2.3:roblox",
			"wally:github.com/pesde-pkg/index:foo/bar@1.2.3:lune",
			"git:github.com/pesde-pkg/index:abcdef+pesde_v1#1.2.3:luau",
			"path:/dev/null:luau",
			"path:filename:with:colons:luau",
		];

		for serialized in ids {
			let id: PackageId = serialized.parse().unwrap();
			assert_eq!(id.to_string(), serialized);
		}
	}
}
