use crate::GixUrl;
#[cfg(feature = "patches")]
use crate::source::ids::PackageId;
use crate::{
	engine::EngineKind,
	manifest::{
		overrides::{OverrideKey, OverrideSpecifier},
		target::Target,
	},
	ser_display_deser_fromstr,
	source::specifiers::DependencySpecifiers,
};
#[cfg(feature = "patches")]
use relative_path::RelativePathBuf;
use semver::VersionReq;
use serde::{Deserialize, Serialize};
use std::{
	collections::{BTreeMap, HashMap},
	fmt::Display,
	hash::Hash,
	str::FromStr,
	sync::Arc,
};
use tracing::instrument;

/// Overrides
pub mod overrides;
/// Targets
pub mod target;

/// A package manifest
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
	/// The description of the package
	#[serde(default)]
	pub description: Option<String>,
	/// The authors of the package
	#[serde(default)]
	pub authors: Vec<String>,
	/// The repository of the package
	#[serde(default)]
	pub repository: Option<url::Url>,
	/// The target of the package
	pub target: Target,
	/// The scripts of the package
	#[serde(default)]
	pub scripts: BTreeMap<String, String>,
	/// The indices to use for the package
	#[serde(default)]
	pub indices: BTreeMap<String, GixUrl>,
	/// The indices to use for the package's wally dependencies
	#[cfg(feature = "wally-compat")]
	#[serde(default)]
	pub wally_indices: BTreeMap<String, GixUrl>,
	/// The overrides this package has
	#[serde(default, skip_serializing)]
	pub overrides: BTreeMap<OverrideKey, OverrideSpecifier>,
	/// The files to include in the package
	#[serde(default)]
	pub includes: Vec<String>,
	/// The patches to apply to packages
	#[cfg(feature = "patches")]
	#[serde(default)]
	pub patches: BTreeMap<Arc<PackageId>, RelativePathBuf>,
	/// A list of globs pointing to workspace members' directories
	#[serde(default)]
	pub workspace_members: Vec<String>,
	/// The Roblox place of this project
	#[serde(default)]
	pub place: BTreeMap<target::RobloxPlaceKind, String>,
	/// The engines this package supports
	#[serde(default)]
	pub engines: BTreeMap<EngineKind, VersionReq>,

	/// The standard dependencies of the package
	#[serde(default, deserialize_with = "crate::util::deserialize_no_dup_keys")]
	pub dependencies: BTreeMap<Alias, DependencySpecifiers>,
	/// The peer dependencies of the package
	#[serde(default, deserialize_with = "crate::util::deserialize_no_dup_keys")]
	pub peer_dependencies: BTreeMap<Alias, DependencySpecifiers>,
	/// The dev dependencies of the package
	#[serde(default, deserialize_with = "crate::util::deserialize_no_dup_keys")]
	pub dev_dependencies: BTreeMap<Alias, DependencySpecifiers>,
	/// An area for user-defined fields, which will always be ignored by pesde
	#[serde(default)]
	pub meta: HashMap<String, toml::Value>,
}

/// An alias of a dependency
/// Equality checks (Ord, PartialOrd, PartialEq, Eq, Hash) are case-insensitive
#[derive(Debug, Clone)]
pub struct Alias(Arc<str>);
ser_display_deser_fromstr!(Alias);

impl Ord for Alias {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.0.to_lowercase().cmp(&other.0.to_lowercase())
	}
}

impl PartialOrd for Alias {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl PartialEq for Alias {
	fn eq(&self, other: &Self) -> bool {
		self.0.to_lowercase() == other.0.to_lowercase()
	}
}

impl Eq for Alias {}

impl Hash for Alias {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.0.to_lowercase().hash(state);
	}
}

impl Display for Alias {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.pad(&self.0)
	}
}

impl FromStr for Alias {
	type Err = errors::AliasFromStr;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if s.is_empty() {
			return Err(errors::AliasFromStr::Empty);
		}

		if s.len() > 48 {
			return Err(errors::AliasFromStr::TooLong(s.to_string()));
		}

		if matches!(
			s.to_ascii_lowercase().as_str(),
			// Windows reserved file names
			"con"    | "prn"  | "aux"  | "nul"  | "com1" | "com2" | "com3" | "com4" | "com5" | "com6" | "com7"
			| "com8" | "com9" | "com¹" | "com²" | "com³" | "lpt1" | "lpt2" | "lpt3" | "lpt4" | "lpt5" | "lpt6"
			| "lpt7" | "lpt8" | "lpt9" | "lpt¹" | "lpt²" | "lpt³"

			// Luau's `@self` alias
			| "self"

			// The Cart runtime (#25)
			| "cart"
		) {
			return Err(errors::AliasFromStr::Reserved(s.to_string()));
		}

		if !s
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
		{
			return Err(errors::AliasFromStr::InvalidCharacters(s.to_string()));
		}

		if EngineKind::from_str(s).is_ok() {
			return Err(errors::AliasFromStr::EngineName(s.to_string()));
		}

		Ok(Self(s.into()))
	}
}

impl Alias {
	/// Get the alias as a string
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

/// A dependency type
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DependencyType {
	/// A standard dependency
	Standard,
	/// A peer dependency
	Peer,
	/// A dev dependency
	Dev,
}

impl DependencyType {
	/// All possible dependency types
	pub const VARIANTS: &'static [DependencyType] = &[
		DependencyType::Standard,
		DependencyType::Peer,
		DependencyType::Dev,
	];
}

impl Manifest {
	/// Get all dependencies from the manifest
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub fn all_dependencies(
		&self,
	) -> Result<BTreeMap<Alias, (DependencySpecifiers, DependencyType)>, errors::AllDependenciesError>
	{
		let mut all_deps = BTreeMap::new();

		for (deps, ty) in [
			(&self.dependencies, DependencyType::Standard),
			(&self.peer_dependencies, DependencyType::Peer),
			(&self.dev_dependencies, DependencyType::Dev),
		] {
			for (alias, spec) in deps {
				if all_deps.insert(alias.clone(), (spec.clone(), ty)).is_some() {
					return Err(errors::AllDependenciesError::AliasConflict(alias.clone()));
				}
			}
		}

		Ok(all_deps)
	}
}

/// Errors that can occur when interacting with manifests
pub mod errors {
	use crate::manifest::Alias;
	use thiserror::Error;

	/// Errors that can occur when parsing an alias from a string
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum AliasFromStr {
		/// The alias is empty
		#[error("the alias is empty")]
		Empty,

		/// The alias has more than 48 characters
		#[error("alias `{0}` has more than 48 characters")]
		TooLong(String),

		/// The alias is reserved
		#[error("alias `{0}` is reserved")]
		Reserved(String),

		/// The alias contains characters outside a-z, A-Z, 0-9, -, and _
		#[error("alias `{0}` contains characters outside a-z, A-Z, 0-9, -, and _")]
		InvalidCharacters(String),

		/// The alias is an engine name
		#[error("alias `{0}` is an engine name")]
		EngineName(String),
	}

	/// Errors that can occur when trying to get all dependencies from a manifest
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum AllDependenciesError {
		/// Another specifier is already using the alias
		#[error("another specifier is already using the alias {0}")]
		AliasConflict(Alias),
	}
}
