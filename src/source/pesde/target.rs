use crate::ser_display_deser_fromstr;
use crate::source::traits::PackageExports;
use relative_path::RelativePathBuf;
use serde::Deserialize;
use serde::Serialize;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

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
	type Err = errors::TargetKindFromStr;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"roblox" => Ok(Self::Roblox),
			"roblox_server" => Ok(Self::RobloxServer),
			"lune" => Ok(Self::Lune),
			"luau" => Ok(Self::Luau),
			t => Err(errors::TargetKindFromStrKind::Unknown(t.to_string()).into()),
		}
	}
}

impl TargetKind {
	/// All possible target variants
	pub const VARIANTS: &'static [TargetKind] = &[
		TargetKind::Roblox,
		TargetKind::RobloxServer,
		TargetKind::Lune,
		TargetKind::Luau,
	];

	/// The directory to store packages in for this target
	/// self is the project's target, dependency is the target of the dependency
	#[must_use]
	pub fn packages_dir(self) -> &'static str {
		// the code below might seem better, but it's just going to create issues with users trying
		// to use a build script, since imports would break between targets

		// if self == dependency {
		//     return "packages".to_string();
		// }

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
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

	/// Converts this target into a [PackageExports]
	#[must_use]
	pub fn into_exports(self) -> PackageExports {
		match self {
			Self::Roblox { lib } | Self::RobloxServer { lib } => PackageExports { lib, bin: None },
			Self::Lune { lib, bin } | Self::Luau { lib, bin } => PackageExports { lib, bin },
		}
	}
}

impl Display for Target {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.kind())
	}
}

/// The kind of a Roblox place property
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum RobloxPlaceKind {
	/// The shared dependencies location
	Shared,
	/// The server dependencies location
	Server,
}

impl TryInto<RobloxPlaceKind> for TargetKind {
	type Error = ();

	fn try_into(self) -> Result<RobloxPlaceKind, Self::Error> {
		match self {
			TargetKind::Roblox => Ok(RobloxPlaceKind::Shared),
			TargetKind::RobloxServer => Ok(RobloxPlaceKind::Server),
			_ => Err(()),
		}
	}
}

impl Display for RobloxPlaceKind {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			RobloxPlaceKind::Shared => write!(f, "shared"),
			RobloxPlaceKind::Server => write!(f, "server"),
		}
	}
}

/// Errors that can occur when working with targets
pub mod errors {
	use thiserror::Error;

	/// Errors that can occur when parsing a target kind from a string
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = TargetKindFromStr))]
	#[non_exhaustive]
	pub enum TargetKindFromStrKind {
		/// The target kind is unknown
		#[error("unknown target kind {0}")]
		Unknown(String),
	}
}
