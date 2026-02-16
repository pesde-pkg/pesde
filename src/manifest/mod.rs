use crate::GixUrl;
use crate::ser_display_deser_fromstr;
use crate::source::DependencySpecifiers;
use crate::source::Realm;
#[cfg(feature = "patches")]
use crate::source::ids::PackageId;
use crate::source::traits::PackageExports;
#[cfg(feature = "patches")]
use relative_path::RelativePathBuf;
use semver::VersionReq;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;
use std::str::FromStr;
use std::sync::Arc;
use tracing::instrument;

/// Indices specified in a manifest
#[derive(Deserialize, Debug, Clone)]
pub struct ManifestIndices {
	/// The indices to use for the package
	#[serde(default, rename = "indices")]
	pub pesde: BTreeMap<String, GixUrl>,
	/// The indices to use for the package's Wally dependencies
	#[serde(default, rename = "wally_indices")]
	pub wally: BTreeMap<String, GixUrl>,
}

/// A specifier for an override
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum OverrideSpecifier {
	/// A specifier for a dependency
	Specifier(DependencySpecifiers),
	/// An alias for a dependency the current project depends on
	Alias(Alias),
}

/// The `workspace` field of the manifest
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ManifestWorkspace {
	/// A list of globs pointing to workspace members' directories
	pub members: Vec<String>,
	/// The patches to apply to packages
	#[cfg(feature = "patches")]
	pub patches: BTreeMap<PackageId, RelativePathBuf>,
	/// The overrides this workspace has
	pub overrides: BTreeMap<PackageId, OverrideSpecifier>,
}

/// The `engines` field of the manifest
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ManifestEngines {
	/// The pesde version this package supports
	pub pesde: Option<VersionReq>,
}

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
	/// The scripts of the package
	#[serde(default)]
	pub scripts: BTreeMap<String, String>,
	/// The indices this package uses
	#[serde(flatten)]
	pub indices: ManifestIndices,
	/// The files to include in the package
	#[serde(default)]
	pub includes: Vec<String>,
	/// The workspace configuration
	#[serde(default)]
	pub workspace: ManifestWorkspace,
	/// The Roblox place of this project
	#[serde(default)]
	pub place: BTreeMap<Realm, String>,
	/// The engines this package supports
	#[serde(default)]
	pub engines: ManifestEngines,
	/// The lib export of this package
	#[serde(default)]
	pub lib: Option<RelativePathBuf>,
	/// The bin export of this package
	#[serde(default)]
	pub bin: Option<RelativePathBuf>,

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
			return Err(errors::AliasFromStrKind::Empty.into());
		}

		if s.len() > 48 {
			return Err(errors::AliasFromStrKind::TooLong(s.to_string()).into());
		}

		if matches!(
			s.to_ascii_lowercase().as_str(),
			// Windows reserved file names
			"con"    | "prn"  | "aux"  | "nul"  | "com1" | "com2" | "com3" | "com4" | "com5" | "com6" | "com7"
			| "com8" | "com9" | "com¹" | "com²" | "com³" | "lpt1" | "lpt2" | "lpt3" | "lpt4" | "lpt5" | "lpt6"
			| "lpt7" | "lpt8" | "lpt9" | "lpt¹" | "lpt²" | "lpt³"

			// Luau's `@self` alias
			| "self"
		) {
			return Err(errors::AliasFromStrKind::Reserved(s.to_string()).into());
		}

		if !s
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
		{
			return Err(errors::AliasFromStrKind::InvalidCharacters(s.to_string()).into());
		}

		if s.eq_ignore_ascii_case("pesde") {
			return Err(errors::AliasFromStrKind::EngineName(s.to_string()).into());
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
					return Err(
						errors::AllDependenciesErrorKind::AliasConflict(alias.clone()).into(),
					);
				}
			}
		}

		Ok(all_deps)
	}

	/// Converts the manifest into a [PackageExports]
	#[must_use]
	pub fn as_exports(&self) -> PackageExports {
		PackageExports {
			lib_file: self.lib.clone(),
			bin_file: self.bin.clone(),
			x_script: self.scripts.get("x").cloned(),
		}
	}
}

/// Errors that can occur when interacting with manifests
pub mod errors {
	use crate::manifest::Alias;
	use thiserror::Error;

	/// Errors that can occur when parsing an alias from a string
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = AliasFromStr))]
	#[non_exhaustive]
	pub enum AliasFromStrKind {
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
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = AllDependenciesError))]
	#[non_exhaustive]
	pub enum AllDependenciesErrorKind {
		/// Another specifier is already using the alias
		#[error("another specifier is already using the alias {0}")]
		AliasConflict(Alias),
	}
}
