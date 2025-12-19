use relative_path::RelativePathBuf;
use semver::VersionReq;
use serde::{Deserialize, Serialize};
use std::fmt::Display;

use crate::{GixUrl, source::DependencySpecifier};

/// A specifier of a Git dependency's version
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GitVersionSpecifier {
	/// A version requirement
	#[serde(rename = "version")]
	VersionReq(VersionReq),
	/// A specific revision
	Rev(String),
}

impl Display for GitVersionSpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::VersionReq(req) => write!(f, "@{req}"),
			Self::Rev(rev) => write!(f, "#{rev}"),
		}
	}
}

/// The specifier for a Git dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct GitDependencySpecifier {
	/// The repository of the package
	pub repo: GixUrl,
	/// The version specifier of the package
	#[serde(flatten)]
	pub version_specifier: GitVersionSpecifier,
	/// The path of the package in the repository
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub path: Option<RelativePathBuf>,
}
impl DependencySpecifier for GitDependencySpecifier {}

impl Display for GitDependencySpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}{}", self.repo.as_url(), self.version_specifier)
	}
}
