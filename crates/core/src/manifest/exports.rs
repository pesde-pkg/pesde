use crate::{
	__private::{is_reserved_file_name, ser_display_deser_fromstr},
	bounded::BoundedSizeError,
};
use serde::{Deserialize, Serialize};
use std::{fmt::Display, ops::Deref, path::Path, str::FromStr};
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

/// An export path. That is, it cannot contain directory references (`.`, `..`),
/// and contains only non-empty Unicode character components.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExportPath(Box<str>);
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

		let mut components = s.split('/');
		for component in components.by_ref().take(MAX_EXPORT_PATH_COMPONENTS) {
			if component.is_empty() {
				return Err(Self::Err::EmptyComponent);
			}

			if component == "."
				|| component == ".."
				|| !component
					.chars()
					.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
				|| is_reserved_file_name(component)
			{
				return Err(Self::Err::InvalidComponent(component.into()));
			}

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

		Ok(Self(s.into()))
	}
}

impl Deref for ExportPath {
	type Target = str;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl AsRef<Path> for ExportPath {
	fn as_ref(&self) -> &Path {
		Path::new(&*self.0)
	}
}

/// Errors that can occur when parsing an export path from a string
#[derive(Debug, Error)]
pub enum ExportPathFromStrError {
	/// The export path is empty
	#[error("the export path is empty")]
	Empty,

	/// The export path has an empty component
	#[error("the export path has an empty component")]
	EmptyComponent,

	/// The export path has a component that is too long
	#[error("the export path has a component that is too long: `{0}`")]
	ComponentTooLong(Box<str>, #[source] BoundedSizeError),

	/// The export path has a component that is invalid (e.g. `.` or `..`)
	#[error("the export path has a component that is invalid: `{0}`")]
	InvalidComponent(Box<str>),

	/// The export path has too many components
	#[error("the export path has too many components")]
	TooManyComponents(#[source] BoundedSizeError),
}
