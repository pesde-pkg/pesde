use crate::ser_display_deser_fromstr;
use crate::source::PackageRefs;
use crate::source::PackageSources;
use crate::source::errors::PackageRefParseError;
use crate::source::errors::PackageSourcesFromStr;
use crate::source::path::PathPackageSource;
use crate::source::path::local_version;
use semver::Version;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

/// A package ID, which is a combination of a name and a version ID
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageId(Arc<(PackageSources, PackageRefs, Version)>);
ser_display_deser_fromstr!(PackageId);

impl PackageId {
	/// Creates a new package ID
	#[must_use]
	pub fn new(source: PackageSources, pkg_ref: PackageRefs, version: Version) -> Self {
		PackageId(Arc::new((source, pkg_ref, version)))
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

	/// Accesses the version
	#[must_use]
	pub fn version(&self) -> &Version {
		&self.0.2
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
			write!(f, "{}:{pkg_ref}", self.source())
		} else {
			let version_sep = match self.source() {
				PackageSources::Git(_) => '#',
				_ => '@',
			};

			write!(
				f,
				"{}:{pkg_ref}{version_sep}{}",
				self.source(),
				self.version()
			)
		}
	}
}

impl FromStr for PackageId {
	type Err = errors::PackageIdParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (tag, s) = s
			.split_once(':')
			.ok_or(errors::PackageIdParseErrorKind::InvalidFormat)?;

		let version_sep = match tag {
			"git" => '#',
			"path" => ':',
			_ => '@',
		};
		let (s, version) = s
			.rsplit_once(version_sep)
			.ok_or(errors::PackageIdParseErrorKind::InvalidFormat)?;

		let (source, pkg_ref) = if tag == "path" {
			("", s)
		} else {
			s.rsplit_once(':')
				.ok_or(errors::PackageIdParseErrorKind::InvalidFormat)?
		};

		let version = if tag == "path" {
			local_version()
		} else {
			version.parse()?
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

		Ok(PackageId::new(source, pkg_ref, version))
	}
}

/// Errors that can occur when working with IDs
pub mod errors {
	use thiserror::Error;

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
		PackageRef(#[from] crate::source::errors::PackageRefParseError),

		/// Parsing the version failed
		#[error("error parsing version")]
		Version(#[from] semver::Error),
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
