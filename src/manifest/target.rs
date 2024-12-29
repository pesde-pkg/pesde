use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{Display, Formatter},
    str::FromStr,
};

/// A kind of target
#[derive(
    SerializeDisplay, DeserializeFromStr, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
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
            t => Err(errors::TargetKindFromStr::Unknown(t.to_string())),
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

    /// The folder to store packages in for this target
    /// self is the project's target, dependency is the target of the dependency
    pub fn packages_folder(&self, dependency: &Self) -> String {
        // the code below might seem better, but it's just going to create issues with users trying
        // to use a build script, since imports would break between targets

        // if self == dependency {
        //     return "packages".to_string();
        // }

        format!("{dependency}_packages")
    }

    /// Returns whether this target is a Roblox target
    pub fn is_roblox(&self) -> bool {
        matches!(self, TargetKind::Roblox | TargetKind::RobloxServer)
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
        /// The files to include in the sync tool's config
        #[serde(default)]
        build_files: BTreeSet<String>,
    },
    /// A Roblox server target
    RobloxServer {
        /// The path to the lib export file
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lib: Option<RelativePathBuf>,
        /// The files to include in the sync tool's config
        #[serde(default)]
        build_files: BTreeSet<String>,
    },
    /// A Lune target
    Lune {
        /// The path to the lib export file
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lib: Option<RelativePathBuf>,
        /// The path to the bin export file
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bin: Option<RelativePathBuf>,
        /// The exported scripts
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        scripts: BTreeMap<String, RelativePathBuf>,
    },
    /// A Luau target
    Luau {
        /// The path to the lib export file
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lib: Option<RelativePathBuf>,
        /// The path to the bin export file
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bin: Option<RelativePathBuf>,
        /// The exported scripts
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        scripts: BTreeMap<String, RelativePathBuf>,
    },
}

impl Target {
    /// Returns the kind of this target
    pub fn kind(&self) -> TargetKind {
        match self {
            Target::Roblox { .. } => TargetKind::Roblox,
            Target::RobloxServer { .. } => TargetKind::RobloxServer,
            Target::Lune { .. } => TargetKind::Lune,
            Target::Luau { .. } => TargetKind::Luau,
        }
    }

    /// Returns the path to the lib export file
    pub fn lib_path(&self) -> Option<&RelativePath> {
        match self {
            Target::Roblox { lib, .. } => lib.as_deref(),
            Target::RobloxServer { lib, .. } => lib.as_deref(),
            Target::Lune { lib, .. } => lib.as_deref(),
            Target::Luau { lib, .. } => lib.as_deref(),
        }
    }

    /// Returns the path to the bin export file
    pub fn bin_path(&self) -> Option<&RelativePath> {
        match self {
            Target::Roblox { .. } => None,
            Target::RobloxServer { .. } => None,
            Target::Lune { bin, .. } => bin.as_deref(),
            Target::Luau { bin, .. } => bin.as_deref(),
        }
    }

    /// Returns the Roblox build files
    pub fn build_files(&self) -> Option<&BTreeSet<String>> {
        match self {
            Target::Roblox { build_files, .. } => Some(build_files),
            Target::RobloxServer { build_files, .. } => Some(build_files),
            _ => None,
        }
    }

    /// Returns the scripts exported by this target
    pub fn scripts(&self) -> Option<&BTreeMap<String, RelativePathBuf>> {
        match self {
            Target::Lune { scripts, .. } => Some(scripts),
            Target::Luau { scripts, .. } => Some(scripts),
            _ => None,
        }
    }
}

impl Display for Target {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind())
    }
}

/// The kind of a Roblox place property
#[derive(
    SerializeDisplay, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd,
)]
#[serde(rename_all = "snake_case")]
pub enum RobloxPlaceKind {
    /// The shared dependencies location
    Shared,
    /// The server dependencies location
    Server,
}

impl TryInto<RobloxPlaceKind> for &TargetKind {
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
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum TargetKindFromStr {
        /// The target kind is unknown
        #[error("unknown target kind {0}")]
        Unknown(String),
    }
}
