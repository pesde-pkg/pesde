use crate::__private::ser_display_deser_fromstr;
use crate::bounded::BoundedSizeError;
use crate::vfs::VfsPath;
use serde::Deserialize;
use serde::Serialize;
use std::fmt::Display;
use std::ops::Deref;
use std::path::Path;
use std::str::FromStr;
use thiserror::Error;

pub const MAX_EXPORT_PATH_COMPONENTS: usize = 255;
pub const MAX_EXPORT_PATH_COMPONENT_LEN: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ManifestExports {
	LibOnly { lib: ExportPath },
	BinOnly { bin: ExportPath },
	LibAndBin { lib: ExportPath, bin: ExportPath },
}
impl ManifestExports {
	/// Returns the lib export path, if it exists
	#[must_use]
	pub fn lib(&self) -> Option<&ExportPath> {
		match self {
			ManifestExports::LibOnly { lib } => Some(lib),
			ManifestExports::BinOnly { .. } => None,
			ManifestExports::LibAndBin { lib, .. } => Some(lib),
		}
	}

	/// Returns the bin export path, if it exists
	#[must_use]
	pub fn bin(&self) -> Option<&ExportPath> {
		match self {
			ManifestExports::LibOnly { .. } => None,
			ManifestExports::BinOnly { bin } => Some(bin),
			ManifestExports::LibAndBin { bin, .. } => Some(bin),
		}
	}
}

/// An export path. It is a [crate::vfs::VfsPath] with additional length restrictions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExportPath(crate::vfs::VfsPath);
ser_display_deser_fromstr!(ExportPath);

impl Display for ExportPath {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

impl FromStr for ExportPath {
	type Err = ExportPathFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if s.is_empty() {
			return Err(Self::Err::Empty);
		}

		let vfs_path = s
			.parse::<crate::vfs::VfsPath>()
			.map_err(|e| Self::Err::InvalidVfsPath(s.into(), e))?;

		let mut components = s.split('/');
		for component in components.by_ref().take(MAX_EXPORT_PATH_COMPONENTS) {
			let len = component.chars().count();
			if len > MAX_EXPORT_PATH_COMPONENT_LEN {
				return Err(Self::Err::ComponentTooLong(
					component.into(),
					BoundedSizeError {
						limit: MAX_EXPORT_PATH_COMPONENT_LEN,
						actual: len,
					},
				));
			}
		}

		let remaining = components.count();
		if remaining > 0 {
			return Err(Self::Err::TooManyComponents(BoundedSizeError {
				limit: MAX_EXPORT_PATH_COMPONENTS,
				actual: MAX_EXPORT_PATH_COMPONENTS + remaining,
			}));
		}

		Ok(Self(vfs_path))
	}
}

impl Deref for ExportPath {
	type Target = VfsPath;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl AsRef<VfsPath> for ExportPath {
	fn as_ref(&self) -> &VfsPath {
		self
	}
}

impl AsRef<Path> for ExportPath {
	fn as_ref(&self) -> &Path {
		self.0.as_ref()
	}
}

/// Errors that can occur when parsing an export path from a string
#[derive(Debug, Error)]
pub enum ExportPathFromStrError {
	/// An export path mustn't point to the root, i.e. be empty
	#[error("the export path must not be empty")]
	Empty,

	/// The path couldn't be parsed as a VFS path
	#[error("the export path `{0}` is not a valid VFS path")]
	InvalidVfsPath(Box<str>, #[source] std::io::Error),

	/// The export path has a component that is too long
	#[error("the export path has a component that is too long: `{0}`")]
	ComponentTooLong(Box<str>, #[source] BoundedSizeError),

	/// The export path has too many components
	#[error("the export path has too many components")]
	TooManyComponents(#[source] BoundedSizeError),
}
