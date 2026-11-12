#![deprecated]

use crate::__private::ser_display_deser_fromstr;
use crate::registry::manifest::ManifestExports;
use serde::Deserialize;
use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::str::FromStr;

use crate::vfs::VfsPath;

/// A kind of target
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TargetKind {
	/// A Roblox target
	Roblox,
	/// A Roblox server target
	RobloxServer,
	/// A Lune target
	Lune,
	/// A Luau target
	Luau,
}
ser_display_deser_fromstr!(TargetKind);

impl Display for TargetKind {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			TargetKind::Roblox => write!(f, "roblox"),
			TargetKind::RobloxServer => write!(f, "roblox_server"),
			TargetKind::Lune => write!(f, "lune"),
			TargetKind::Luau => write!(f, "luau"),
		}
	}
}

impl FromStr for TargetKind {
	type Err = Box<str>;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"roblox" => Ok(Self::Roblox),
			"roblox_server" => Ok(Self::RobloxServer),
			"lune" => Ok(Self::Lune),
			"luau" => Ok(Self::Luau),
			t => Err(format!("unknown target kind: {t}").into()),
		}
	}
}

impl TargetKind {
	/// The directory to store packages in for this target
	#[must_use]
	pub const fn packages_dir(self) -> &'static str {
		match self {
			Self::Luau => "luau_packages",
			Self::Lune => "lune_packages",
			Self::Roblox => "roblox_packages",
			Self::RobloxServer => "roblox_server_packages",
		}
	}

	/// Returns whether this target is a Roblox target
	#[must_use]
	pub fn is_roblox(self) -> bool {
		matches!(self, TargetKind::Roblox | TargetKind::RobloxServer)
	}

	/// Returns whether this target supports bin exports
	#[must_use]
	pub fn has_bin(self) -> bool {
		!self.is_roblox()
	}
}

/// A target of a package
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case", tag = "environment")]
pub enum Target {
	/// A Roblox target
	Roblox {
		/// The path to the lib export file
		#[serde(default, skip_serializing_if = "Option::is_none")]
		lib: Option<RelativePathBuf>,
	},
	/// A Roblox server target
	RobloxServer {
		/// The path to the lib export file
		#[serde(default, skip_serializing_if = "Option::is_none")]
		lib: Option<RelativePathBuf>,
	},
	/// A Lune target
	Lune {
		/// The path to the lib export file
		#[serde(default, skip_serializing_if = "Option::is_none")]
		lib: Option<RelativePathBuf>,
		/// The path to the bin export file
		#[serde(default, skip_serializing_if = "Option::is_none")]
		bin: Option<RelativePathBuf>,
	},
	/// A Luau target
	Luau {
		/// The path to the lib export file
		#[serde(default, skip_serializing_if = "Option::is_none")]
		lib: Option<RelativePathBuf>,
		/// The path to the bin export file
		#[serde(default, skip_serializing_if = "Option::is_none")]
		bin: Option<RelativePathBuf>,
	},
}

impl Target {
	/// Returns the kind of this target
	#[must_use]
	pub fn kind(&self) -> TargetKind {
		match self {
			Target::Roblox { .. } => TargetKind::Roblox,
			Target::RobloxServer { .. } => TargetKind::RobloxServer,
			Target::Lune { .. } => TargetKind::Lune,
			Target::Luau { .. } => TargetKind::Luau,
		}
	}

	pub fn exports(&self) -> Result<ManifestExports, Box<dyn Error + Send + Sync>> {
		let lib = match &self {
			Target::Luau { lib, .. } => lib.as_ref(),
			Target::Lune { lib, .. } => lib.as_ref(),
			Target::Roblox { lib, .. } => lib.as_ref(),
			Target::RobloxServer { lib, .. } => lib.as_ref(),
		}
		.map(|l| l.0.parse().map_err(|e| format!("invalid lib path: {e}")))
		.transpose()?;

		let bin = match &self {
			Target::Luau { bin, .. } => bin.as_ref(),
			Target::Lune { bin, .. } => bin.as_ref(),
			Target::Roblox { .. } => None,
			Target::RobloxServer { .. } => None,
		}
		.map(|b| b.0.parse().map_err(|e| format!("invalid bin path: {e}")))
		.transpose()?;

		let exports = match (lib, bin) {
			(Some(lib), Some(bin)) => ManifestExports::LibAndBin { lib, bin },
			(Some(lib), None) => ManifestExports::LibOnly { lib },
			(None, Some(bin)) => ManifestExports::BinOnly { bin },
			(None, None) => {
				return Err("neither lib nor bin export is specified for target {}".into());
			}
		};

		Ok(exports)
	}
}

impl Display for Target {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.kind())
	}
}

/// A serde adapter for legacy relative paths that could be non-incremental and non-normalized.
#[derive(Debug, Clone)]
pub struct RelativePathBuf(pub VfsPath);

impl Default for RelativePathBuf {
	fn default() -> Self {
		Self(VfsPath::root())
	}
}

impl Display for RelativePathBuf {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl<'de> Deserialize<'de> for RelativePathBuf {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct Visitor;
		impl serde::de::Visitor<'_> for Visitor {
			type Value = RelativePathBuf;

			fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
				write!(formatter, "a relative path")
			}

			fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
			where
				E: serde::de::Error,
			{
				let path = Path::new(v);
				let path = VfsPath::from_path(path)
					.map_err(|e| E::custom(format!("invalid path: {e}")))?;
				Ok(RelativePathBuf(path))
			}
		}

		deserializer.deserialize_str(Visitor)
	}
}
