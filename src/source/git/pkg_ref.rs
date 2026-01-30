use std::str::FromStr;

use relative_path::RelativePathBuf;

use crate::ser_display_deser_fromstr;
use crate::source::PackageRef;
use crate::source::StructureKind;

/// A Git package reference
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitPackageRef {
	/// The structure kind of this package
	pub structure_kind: StructureKind,
	/// The id of the package's root tree
	pub tree_id: String,
	/// The path to the package within the tree
	pub path: RelativePathBuf,
}
ser_display_deser_fromstr!(GitPackageRef);

impl PackageRef for GitPackageRef {
	fn structure_kind(&self) -> StructureKind {
		self.structure_kind
	}
}

impl std::fmt::Display for GitPackageRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let path = if self.path.as_str().is_empty() {
			format_args!("")
		} else {
			format_args!("+{}", self.path)
		};
		write!(f, "{}+{}{path}", self.structure_kind, self.tree_id)
	}
}

/// Errors that can occur when parsing a Git package reference
pub type GitPackageRefParseError = crate::source::errors::GitPackageRefParseError;

impl FromStr for GitPackageRef {
	type Err = GitPackageRefParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let mut s = s.split('+');
		let structure_kind = s
			.next()
			.ok_or(crate::source::errors::GitPackageRefParseErrorKind::InvalidFormat)?;
		let tree_id = s
			.next()
			.ok_or(crate::source::errors::GitPackageRefParseErrorKind::InvalidFormat)?;
		let path = s.next().map(Into::into).unwrap_or_default();

		Ok(GitPackageRef {
			structure_kind: structure_kind.parse()?,
			tree_id: tree_id.to_string(),
			path,
		})
	}
}
