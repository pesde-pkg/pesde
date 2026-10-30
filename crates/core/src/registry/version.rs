use std::fmt::Display;
use std::str::FromStr;

use semver::{Prerelease, Version};
use thiserror::Error;

use crate::__private::ser_display_deser_fromstr;
use crate::bounded::BoundedSizeError;

const PRE_MAX: usize = 10;

/// An opinionated subset of (Cargo) SemVer.
/// Differences from [Version]:
/// - build metadata is not allowed: it is ambiguous (can't specify it) and overall has little to no purpose
/// - only lowercase ASCII is allowed: while without this requirement versions can be deterministically chosen, they are surprising to users: `1.2.3-hello` is not `1.2.3-Hello`
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PesdeStyleVersion {
	/// [Version::major]
	pub major: u64,
	/// [Version::minor]
	pub minor: u64,
	/// [Version::patch]
	pub patch: u64,
	/// [Version::pre]
	pre: Prerelease,
}
ser_display_deser_fromstr!(PesdeStyleVersion);

impl PesdeStyleVersion {
	pub fn new(version: Version) -> Result<Self, PesdeStyleVersionError> {
		if !version.build.is_empty() {
			return Err(PesdeStyleVersionError::HasBuildMetadata);
		}

		if version.pre.chars().any(|c| c.is_ascii_uppercase()) {
			return Err(PesdeStyleVersionError::UpperPrerelease);
		}
		let len = version.pre.chars().count();
		if len > PRE_MAX {
			return Err(PesdeStyleVersionError::PrereleaseLength(BoundedSizeError {
				limit: PRE_MAX,
				actual: len,
			}));
		}

		Ok(Self {
			major: version.major,
			minor: version.minor,
			patch: version.patch,
			pre: version.pre,
		})
	}

	/// [Version::pre]
	#[must_use]
	pub fn pre(&self) -> &Prerelease {
		&self.pre
	}
}

impl Display for PesdeStyleVersion {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let pre = if self.pre.is_empty() {
			format_args!("")
		} else {
			format_args!("-{}", self.pre)
		};

		write!(f, "{}.{}.{}{}", self.major, self.minor, self.patch, pre)
	}
}

impl FromStr for PesdeStyleVersion {
	type Err = PesdeStyleVersionFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		PesdeStyleVersion::new(Version::parse(s)?).map_err(Into::into)
	}
}

/// Errors that can occur when creating a [PesdeStyleVersion]
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PesdeStyleVersionError {
	/// The version contained build metadata
	#[error("pesde style versions mustn't contain build metadata")]
	HasBuildMetadata,

	/// The version's prerelease wasn't lowercase
	#[error("pesde style versions' prereleases must be lowercase")]
	UpperPrerelease,

	/// The version's prerelease was too long
	#[error("pesde style versions' prereleases must be shorter")]
	PrereleaseLength(#[source] BoundedSizeError),
}

/// Errors that can occur when parsing a [PesdeStyleVersion] from str
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PesdeStyleVersionFromStrError {
	/// The version was not valid SemVer
	#[error("failed to parse input as semver")]
	SemVer(#[from] semver::Error),

	#[error(transparent)]
	Pesde(#[from] PesdeStyleVersionError),
}

#[cfg(test)]
mod tests {
	use semver::BuildMetadata;

	use super::*;

	// Versions should be small. They could be wrapped in Bounded, but this is guaranteed by its
	// invariants. Additionally, we don't want make 255 an official contract.
	#[test]
	fn versions_are_small() {
		assert!(
			PesdeStyleVersion::new(Version {
				major: u64::MAX,
				minor: u64::MAX,
				patch: u64::MAX,
				pre: Prerelease::new(&"a".repeat(PRE_MAX)).unwrap(),
				build: BuildMetadata::EMPTY
			})
			.unwrap()
			.to_string()
			.len() < 255
		);
	}
}
