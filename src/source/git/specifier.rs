use relative_path::RelativePathBuf;
use serde::{Deserialize, Serialize};
use std::fmt::Display;

use crate::source::DependencySpecifier;

/// The specifier for a Git dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GitDependencySpecifier {
	/// The repository of the package
	#[serde(
		serialize_with = "crate::util::serialize_gix_url",
		deserialize_with = "crate::util::deserialize_git_like_url"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "url::Url"))]
	pub repo: gix::Url,
	/// The revision of the package
	pub rev: String,
	/// The path of the package in the repository
	#[serde(default, skip_serializing_if = "Option::is_none")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<std::path::PathBuf>"))]
	pub path: Option<RelativePathBuf>,
}
impl DependencySpecifier for GitDependencySpecifier {}

impl Display for GitDependencySpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}#{}", self.repo, self.rev)
	}
}
