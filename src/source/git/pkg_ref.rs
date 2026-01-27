use std::str::FromStr;

use crate::ser_display_deser_fromstr;
use crate::source::PackageRef;
use crate::source::refs::StructureKind;

/// A Git package reference
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitPackageRef {
	/// The id of the package's tree
	pub tree_id: String,
	/// The structure kind of this package
	pub structure_kind: StructureKind,
}
ser_display_deser_fromstr!(GitPackageRef);

impl PackageRef for GitPackageRef {
	fn structure_kind(&self) -> StructureKind {
		self.structure_kind
	}
}

impl std::fmt::Display for GitPackageRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}+{}", self.tree_id, self.structure_kind)
	}
}

impl FromStr for GitPackageRef {
	type Err = crate::source::refs::errors::GitPackageRefParseError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let Some((tree_id, structure_kind)) = s.split_once('+') else {
			return Err(
				crate::source::refs::errors::GitPackageRefParseErrorKind::InvalidFormat.into(),
			);
		};
		Ok(GitPackageRef {
			tree_id: tree_id.to_string(),
			structure_kind: structure_kind.parse()?,
		})
	}
}
