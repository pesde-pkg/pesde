use crate::__private::is_fs_safe_path_component;
use crate::__private::ser_display_deser_fromstr;
use std::fmt::Display;
use std::ops::Deref;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VfsPath(Arc<str>);
ser_display_deser_fromstr!(VfsPath);

fn valid_vfs_component(component: &str) -> std::io::Result<()> {
	if component.is_empty() {
		return Err(std::io::Error::new(
			std::io::ErrorKind::InvalidInput,
			"VfsPath component cannot be empty",
		));
	}

	if component == "." || component == ".." || !is_fs_safe_path_component(component) {
		return Err(std::io::Error::new(
			std::io::ErrorKind::InvalidInput,
			format!("VfsPath component `{component}` contains invalid characters"),
		));
	}

	Ok(())
}

impl VfsPath {
	#[must_use]
	pub fn root() -> Self {
		Self(Arc::from(""))
	}

	#[must_use]
	pub fn join(&self, other: &VfsPath) -> VfsPath {
		if self.is_empty() {
			return other.clone();
		} else if other.is_empty() {
			return self.clone();
		}

		let mut new_path = self.0.to_string();
		if !new_path.is_empty() {
			new_path.push('/');
		}
		new_path.push_str(&other.0);
		VfsPath(Arc::from(new_path))
	}

	/// Converts a relative path to a `VfsPath`. Unlike `FromStr`,
	/// this function will work with Windows separators. It will also
	/// appropriately handle `.` and `..` components.
	pub fn from_path(rel_path: impl AsRef<Path>) -> std::io::Result<Self> {
		let mut path = String::new();

		for component in rel_path.as_ref().components() {
			let component = match component {
				std::path::Component::CurDir => continue,
				std::path::Component::ParentDir => {
					if let Some(pos) = path.rfind('/') {
						path.truncate(pos);
						continue;
					}
					return Err(std::io::Error::new(
						std::io::ErrorKind::InvalidInput,
						"VfsPath cannot go above root",
					));
				}
				std::path::Component::Normal(component) => component,
				_ => {
					return Err(std::io::Error::new(
						std::io::ErrorKind::InvalidInput,
						format!("VfsPath component `{component:?}` is not a valid path component"),
					));
				}
			};

			let component = component
				.to_str()
				.ok_or_else(|| {
					std::io::Error::new(
						std::io::ErrorKind::InvalidInput,
						"VfsPath component is not valid UTF-8",
					)
				})?
				.parse::<VfsPath>()
				.map_err(std::io::Error::other)?;
			if !path.is_empty() {
				path.push('/');
			}
			path.push_str(&component);
		}

		Ok(Self(Arc::from(path)))
	}

	#[must_use]
	pub fn pop(&self) -> Option<(VfsPath, VfsPath)> {
		if self.is_empty() {
			return None;
		}

		if let Some((parent, file_name)) = self.0.rsplit_once('/') {
			Some((VfsPath(Arc::from(parent)), VfsPath(Arc::from(file_name))))
		} else {
			Some((VfsPath::root(), self.clone()))
		}
	}

	/// Returns a relative path (including parent directory components) to get to `other` from `self`.
	/// If `other` is not a descendant of `self`, returns the full path to `other`.
	#[must_use]
	pub fn diff(&self, other: &VfsPath) -> Box<str> {
		let mut from = self.split('/').filter(|c| !c.is_empty()).peekable();
		let mut to = other.split('/').filter(|c| !c.is_empty()).peekable();

		while let (Some(a), Some(b)) = (from.peek(), to.peek()) {
			if a != b {
				break;
			}
			from.next();
			to.next();
		}

		let mut path = String::new();
		for _ in from {
			path.push_str("../");
		}

		if path.is_empty() {
			path.push_str("./");
		}

		for (i, component) in to.enumerate() {
			if i > 0 {
				path.push('/');
			}
			path.push_str(component);
		}

		if path.ends_with('/') && path.len() > 3 {
			path.pop();
		}

		path.into()
	}
}

impl FromStr for VfsPath {
	type Err = std::io::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if !s.is_empty() {
			for component in s.split('/') {
				valid_vfs_component(component)?;
			}
		}

		Ok(Self(s.into()))
	}
}

impl Display for VfsPath {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

impl Deref for VfsPath {
	type Target = str;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl AsRef<Path> for VfsPath {
	fn as_ref(&self) -> &Path {
		Path::new(&**self)
	}
}
