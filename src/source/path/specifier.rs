use crate::source::DependencySpecifier;
use serde::{Deserialize, Serialize};
use std::{fmt::Display, path::PathBuf};

/// The specifier for a path dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct PathDependencySpecifier {
	/// The path to the package
	pub path: PathBuf,
}
impl DependencySpecifier for PathDependencySpecifier {}

impl Display for PathDependencySpecifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "path:{}", self.path.display())
	}
}
