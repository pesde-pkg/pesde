use std::str::FromStr;

use crate::ser_display_deser_fromstr;
use crate::source::PackageRef;

/// A Git package reference
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitPackageRef {
	/// The id of the package's tree
	pub tree_id: String,
}
ser_display_deser_fromstr!(GitPackageRef);

impl PackageRef for GitPackageRef {}

impl std::fmt::Display for GitPackageRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.tree_id)
	}
}

/// Errors that can occur when parsing a Git package reference
#[derive(Debug, thiserror::Error)]
pub enum GitPackageRefParseError {}

impl FromStr for GitPackageRef {
	type Err = GitPackageRefParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(GitPackageRef {
			tree_id: s.to_string(),
		})
	}
}
