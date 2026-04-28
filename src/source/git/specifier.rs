//! Git dependency specifier
use relative_path::RelativePathBuf;
use serde::Deserialize;
use serde::Serialize;
use std::fmt::Display;

use crate::GixUrl;
use crate::source::DependencySpecifier;
use crate::source::Realm;

/// The field that discriminates Git dependencies from other dependencies
pub const DISCRIMINATOR_FIELD: &str = "repo";

/// The specifier for a Git dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct GitDependencySpecifier {
	/// The repository of the package
	pub repo: GixUrl,
	/// The revision of the package, can be a branch, tag or commit hash
	pub rev: String,
	/// The path of the package in the repository
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub path: Option<RelativePathBuf>,
	/// The realm of the package
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub realm: Option<Realm>,
}
impl DependencySpecifier for GitDependencySpecifier {
	fn realm(&self) -> Option<Realm> {
		self.realm
	}
}

impl Display for GitDependencySpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}#{}", self.repo, self.rev)
	}
}

fn deserialize_gix_url<'de, D>(deserializer: D) -> Result<GixUrl, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let s = String::deserialize(deserializer)?;
	s.try_into()
		.map(GixUrl::new)
		.map_err(serde::de::Error::custom)
}

/// The specifier for a Git dependency in the index
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct IndexGitDependencySpecifier {
	/// The repository of the package
	#[serde(deserialize_with = "deserialize_gix_url")]
	pub repo: GixUrl,
	/// The version specifier of the package
	pub rev: String,
	/// The path of the package in the repository
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub path: Option<RelativePathBuf>,
}
