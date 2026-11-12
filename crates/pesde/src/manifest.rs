#![expect(deprecated)]
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::Url;
use crate::bounded::BoundedBTreeMap;
use crate::bounded::BoundedSizeError;
use crate::bounded::BoundedStr;
use crate::bounded::BoundedUrl;
use crate::registry::PackageName;
use crate::registry::PackageNameFromStrError;
use crate::registry::PesdeStyleVersion;
use crate::registry::manifest::Alias;
use crate::registry::manifest::DependencyType;
use crate::registry::manifest::MAX_DEPENDENCIES;
use crate::registry::manifest::MAX_DESCRIPTION_LEN;
use crate::registry::manifest::MAX_LICENSE_LEN;
use crate::registry::manifest::MAX_URL_LEN;
use crate::registry::manifest::ManifestExports;
use crate::registry::manifest::ManifestPackage;
use crate::registry::manifest::ResolvedDependencySpecifier;
use crate::registry::manifest::ResolvedManifest;
use crate::registry::manifest::config_luau::LuauConfig;
use crate::registry::manifest::wally_compat::WallyPackageNameFromStrError;
use crate::semver;
use serde::Deserialize;
use serde::de::value::StrDeserializer;
use thiserror::Error;

use crate::scripts::hooks::Hooks;
use crate::source::DependencySpecifier;
use crate::source::GitDependencySpecifier;
use crate::source::LegacyPesdeDependencySpecifier;
use crate::source::PackageId;
use crate::source::PesdeDependencySpecifier;
use crate::source::WallyDependencySpecifier;
use crate::source::WorkspaceDependencySpecifier;
use crate::source::WorkspaceVersionSpecifier;
use crate::vfs::glob::VfsGlobset;

/// The `workspace` field of the manifest
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ManifestWorkspace {
	/// A list of globs pointing to workspace members' directories
	pub members: VfsGlobset,
	// /// The patches to apply to packages
	// pub patches: BTreeMap<PackageId, RelativePathBuf>,
	/// The overrides this workspace has
	pub overrides: BTreeMap<PackageId, DependencySpecifier>,
}

/// An intermediary representation of a manifest, containing the
/// [crate::registry::manifest::ResolvedManifest] information, but
/// also data we care about in the client context.
#[derive(Debug)]
#[non_exhaustive]
pub struct Manifest {
	/// The package information
	pub package: ManifestPackage,
	pub workspace: ManifestWorkspace,
	pub publish: bool,
	/// The files to be included in the package
	pub includes: VfsGlobset,
	/// The exports of the package
	pub exports: Option<ManifestExports>,
	/// The Luau configuration of the package
	pub config: LuauConfig,
	/// The hooks of the package
	pub hooks: Hooks,
	/// The scripts of the package
	pub scripts: BTreeMap<Box<str>, Box<str>>,
	/// The dependencies of the package
	pub dependencies:
		BoundedBTreeMap<Alias, (DependencySpecifier, DependencyType), MAX_DEPENDENCIES>,
	sources: BTreeMap<Box<str>, SourceSpecifier>,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ManifestParseError {
	#[error("invalid TOML")]
	InvalidToml(#[from] toml::de::Error),

	#[error("unknown source `{0}`")]
	UnknownSource(Box<str>),

	#[error("malformed pesde dependency specifier `{0}`")]
	MalformedPesdeDependencySpecifier(Box<str>),

	#[error("malformed wally dependency specifier `{0}`")]
	MalformedWallyDependencySpecifier(Box<str>),

	#[error("malformed legacy pesde dependency specifier `{0}`")]
	MalformedLegacyPesdeDependencySpecifier(Box<str>),

	#[error("malformed package name")]
	InvalidPesdePackageName(#[from] PackageNameFromStrError),

	#[error("malformed wally package name")]
	InvalidWallyPackageName(#[from] WallyPackageNameFromStrError),

	#[error("malformed version requirement")]
	InvalidVersionReq(#[from] semver::Error),

	#[deprecated]
	#[error("malformed target kind: `{0}`")]
	InvalidTargetKind(Box<str>),

	#[error("malformed git dependency path `{0}`")]
	InvalidGitDependencyPath(Box<str>, #[source] std::io::Error),

	#[error("malformed workspace version specifier: `{0}`")]
	InvalidWorkspaceVersionSpecifier(Box<str>),

	#[error("unknown dependency alias `{0}`")]
	UnknownDependencyAlias(Alias),

	#[error("duplicate override for package `{0}`")]
	DuplicateOverride(PackageId),

	#[error("duplicate dependency alias `{0}`")]
	DuplicateDependencyAlias(Alias),

	#[error("too many dependencies (max {MAX_DEPENDENCIES})")]
	TooManyDependencies(#[source] BoundedSizeError),
}

// not sure whether this should be part of the public api, especially in its current form
#[doc(hidden)]
pub fn parse_manifest(
	local: &str,
	_root: Option<&Manifest>,
) -> Result<Manifest, ManifestParseError> {
	fn true_bool() -> bool {
		true
	}

	#[derive(Deserialize, Debug)]
	struct UserManifestPackage {
		name: PackageName,
		version: PesdeStyleVersion,
		#[serde(default = "true_bool")]
		publish: bool,
		#[serde(default)]
		description: BoundedStr<MAX_DESCRIPTION_LEN>,
		#[serde(default)]
		license: BoundedStr<MAX_LICENSE_LEN>,
		#[serde(default)]
		repository: Option<BoundedUrl<MAX_URL_LEN>>,
		#[serde(default)]
		includes: VfsGlobset,
	}

	#[derive(Deserialize, Debug, Default)]
	#[serde(default)]
	struct UserManifestWorkspace {
		members: VfsGlobset,
		// patches: BTreeMap<PackageId, RelativePathBuf>,
		overrides: BTreeMap<PackageId, OverrideSpecifier>,
	}

	#[derive(Deserialize, Debug)]
	#[serde(untagged)]
	enum OverrideSpecifier {
		Alias(Alias),
		Specifier(DependencySpecifier),
	}

	fn default_sources() -> BTreeMap<Box<str>, SourceSpecifier> {
		BTreeMap::from([("workspace".into(), SourceSpecifier::Workspace)])
	}

	fn deserialize_sources<'de, D>(
		deserializer: D,
	) -> Result<BTreeMap<Box<str>, SourceSpecifier>, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct Visitor;
		impl<'de> serde::de::Visitor<'de> for Visitor {
			type Value = BTreeMap<Box<str>, SourceSpecifier>;

			fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
				write!(formatter, "a map of sources")
			}

			fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
			where
				A: serde::de::MapAccess<'de>,
			{
				let mut result = default_sources();

				while let Some((key, value)) = map.next_entry::<Box<str>, _>()? {
					if &*key == "workspace" {
						return Err(serde::de::Error::custom(
							"`workspace` is a reserved source name",
						));
					}

					match result.entry(key) {
						std::collections::btree_map::Entry::Vacant(entry) => {
							entry.insert(value);
						}
						std::collections::btree_map::Entry::Occupied(entry) => {
							return Err(serde::de::Error::custom(format!(
								"duplicate source `{}`",
								entry.key()
							)));
						}
					}
				}

				Ok(result)
			}
		}

		deserializer.deserialize_map(Visitor)
	}

	#[derive(Deserialize, Debug)]
	struct UserManifest {
		package: UserManifestPackage,
		#[serde(default)]
		exports: Option<ManifestExports>,
		#[serde(default)]
		config: LuauConfig,
		#[serde(default)]
		workspace: UserManifestWorkspace,
		#[serde(default)]
		hooks: Hooks,
		#[serde(default)]
		scripts: BTreeMap<Box<str>, Box<str>>,

		#[serde(default = "default_sources", deserialize_with = "deserialize_sources")]
		sources: BTreeMap<Box<str>, SourceSpecifier>,
		#[serde(default)]
		dependencies: BTreeMap<Alias, Box<str>>,
		#[serde(default)]
		peer_dependencies: BTreeMap<Alias, Box<str>>,
		#[serde(default)]
		dev_dependencies: BTreeMap<Alias, Box<str>>,

		#[serde(rename = "meta", default)]
		_meta: toml::Table,
	}

	let deserializer = toml::Deserializer::parse(local).map_err(ManifestParseError::InvalidToml)?;
	let user_manifest: UserManifest = serde_ignored::deserialize(deserializer, |key| {
		// not erroring to make sure additive updates aren't painful
		tracing::warn!("unknown field in manifest: `{key}`");
	})
	.map_err(ManifestParseError::InvalidToml)?;

	let user_dependencies = user_manifest
		.dependencies
		.iter()
		.zip(std::iter::repeat(DependencyType::Standard))
		.chain(
			user_manifest
				.peer_dependencies
				.iter()
				.zip(std::iter::repeat(DependencyType::Peer)),
		)
		.chain(
			user_manifest
				.dev_dependencies
				.iter()
				.zip(std::iter::repeat(DependencyType::Dev)),
		);

	let mut dependencies = BTreeMap::new();
	for ((alias, specifier), ty) in user_dependencies {
		let (source, data) = specifier.split_once(':').unwrap_or(("default", specifier));
		let source = user_manifest
			.sources
			.get(source)
			.ok_or_else(|| ManifestParseError::UnknownSource(source.into()))?;
		let specifier = parse_specifier(source, data)?;

		if dependencies
			.insert(alias.clone(), (specifier, ty))
			.is_some()
		{
			return Err(ManifestParseError::DuplicateDependencyAlias(alias.clone()));
		}
	}

	let mut workspace = ManifestWorkspace {
		members: user_manifest.workspace.members,
		overrides: BTreeMap::new(),
	};

	for (id, specifier) in user_manifest.workspace.overrides {
		let specifier = match specifier {
			OverrideSpecifier::Alias(alias) => {
				let (specifier, _) = dependencies
					.get(&alias)
					.ok_or_else(|| ManifestParseError::UnknownDependencyAlias(alias.clone()))?;
				specifier.clone()
			}
			OverrideSpecifier::Specifier(specifier) => specifier,
		};

		match workspace.overrides.entry(id) {
			std::collections::btree_map::Entry::Vacant(entry) => {
				entry.insert(specifier);
			}
			std::collections::btree_map::Entry::Occupied(entry) => {
				return Err(ManifestParseError::DuplicateOverride(entry.key().clone()));
			}
		}
	}

	Ok(Manifest {
		package: ManifestPackage {
			name: user_manifest.package.name,
			version: user_manifest.package.version,
			description: user_manifest.package.description,
			license: user_manifest.package.license,
			repository: user_manifest.package.repository,
		},
		workspace,
		publish: user_manifest.package.publish,
		includes: user_manifest.package.includes,
		exports: user_manifest.exports,
		config: user_manifest.config,
		hooks: user_manifest.hooks,
		scripts: user_manifest.scripts,
		// absolute_paths: user_manifest.absolute_paths,
		dependencies: BoundedBTreeMap::new(dependencies)
			.map_err(ManifestParseError::TooManyDependencies)?,
		sources: user_manifest.sources,
	})
}

fn parse_specifier(
	source: &SourceSpecifier,
	data: &str,
) -> Result<DependencySpecifier, ManifestParseError> {
	Ok(match source {
		SourceSpecifier::Pesde(registry) => {
			let (name, version) = data.split_once('@').ok_or_else(|| {
				ManifestParseError::MalformedPesdeDependencySpecifier(data.into())
			})?;

			DependencySpecifier::Pesde(PesdeDependencySpecifier {
				name: name
					.parse()
					.map_err(ManifestParseError::InvalidPesdePackageName)?,
				req: version
					.parse()
					.map_err(ManifestParseError::InvalidVersionReq)?,
				registry: registry.clone().into_inner(),
			})
		}
		SourceSpecifier::Wally(registry) => {
			let (name, version) = data.split_once('@').ok_or_else(|| {
				ManifestParseError::MalformedWallyDependencySpecifier(data.into())
			})?;

			DependencySpecifier::Wally(WallyDependencySpecifier {
				name: name
					.parse()
					.map_err(ManifestParseError::InvalidWallyPackageName)?,
				req: version
					.parse()
					.map_err(ManifestParseError::InvalidVersionReq)?,
				index: registry.clone().into_inner(),
			})
		}
		SourceSpecifier::LegacyPesde(registry) => {
			let (name, rest) = data.split_once('@').ok_or_else(|| {
				ManifestParseError::MalformedLegacyPesdeDependencySpecifier(data.into())
			})?;
			let (version, target) = rest.split_once(' ').ok_or_else(|| {
				ManifestParseError::MalformedLegacyPesdeDependencySpecifier(data.into())
			})?;

			DependencySpecifier::LegacyPesde(LegacyPesdeDependencySpecifier {
				name: name
					.parse()
					.map_err(ManifestParseError::InvalidPesdePackageName)?,
				req: version
					.parse()
					.map_err(ManifestParseError::InvalidVersionReq)?,
				target: target
					.parse()
					.map_err(ManifestParseError::InvalidTargetKind)?,
				index: registry.clone().into_inner(),
			})
		}
		SourceSpecifier::Git(url, rev) => {
			let path = data
				.parse()
				.map_err(|e| ManifestParseError::InvalidGitDependencyPath(data.into(), e))?;

			DependencySpecifier::Git(GitDependencySpecifier {
				url: url.clone().into_inner(),
				rev: rev.clone(),
				path,
			})
		}
		SourceSpecifier::Workspace => {
			let parts = data.split_once('@');
			let name = parts
				.map_or(data, |(n, _)| n)
				.parse()
				.map_err(ManifestParseError::InvalidPesdePackageName)?;

			let version_specifier =
				parts.map_or(Ok(WorkspaceVersionSpecifier::default()), |(_, v)| {
					v.parse()
						.map_err(ManifestParseError::InvalidWorkspaceVersionSpecifier)
				})?;

			DependencySpecifier::Workspace(WorkspaceDependencySpecifier {
				name,
				version_specifier,
			})
		}
	})
}

impl Manifest {
	// doesn't expose SourceSpecifier because that would be a leaky abstraction, used by
	// e.g. the CLI to allow users to specify named specifiers as arguments
	pub fn parse_specifier(
		&self,
		source: &str,
		specifier: &str,
	) -> Result<Option<DependencySpecifier>, ManifestParseError> {
		let Some(source) = self.sources.get(source) else {
			return Ok(None);
		};

		parse_specifier(source, specifier).map(Some)
	}
}

#[derive(Debug)]
enum SourceSpecifier {
	Pesde(BoundedUrl<MAX_URL_LEN>),
	Wally(BoundedUrl<MAX_URL_LEN>),
	LegacyPesde(BoundedUrl<MAX_URL_LEN>),
	Git(BoundedUrl<MAX_URL_LEN>, Arc<str>),
	Workspace,
}

impl<'de> Deserialize<'de> for SourceSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct Visitor;
		impl<'de> serde::de::Visitor<'de> for Visitor {
			type Value = SourceSpecifier;

			fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
				write!(formatter, "a source specifier")
			}

			fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
			where
				E: serde::de::Error,
			{
				Ok(SourceSpecifier::Pesde(BoundedUrl::deserialize(
					StrDeserializer::new(v),
				)?))
			}

			fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
			where
				A: serde::de::MapAccess<'de>,
			{
				#[derive(Deserialize)]
				#[serde(tag = "kind", rename_all = "snake_case")]
				enum Raw {
					Wally {
						url: BoundedUrl<MAX_URL_LEN>,
					},
					LegacyPesde {
						url: BoundedUrl<MAX_URL_LEN>,
					},
					Git {
						url: BoundedUrl<MAX_URL_LEN>,
						rev: Arc<str>,
					},
				}

				let raw = serde::de::Deserialize::deserialize(
					serde::de::value::MapAccessDeserializer::new(map),
				)?;
				match raw {
					Raw::Wally { url } => Ok(SourceSpecifier::Wally(url)),
					Raw::LegacyPesde { url } => Ok(SourceSpecifier::LegacyPesde(url)),
					Raw::Git { url, rev } => Ok(SourceSpecifier::Git(url, rev)),
				}
			}
		}

		deserializer.deserialize_any(Visitor)
	}
}

impl Manifest {
	#[must_use]
	pub fn new_empty(package: ManifestPackage) -> Self {
		Self {
			package,
			workspace: Default::default(),
			publish: false,
			includes: Default::default(),
			exports: None,
			config: Default::default(),
			hooks: Default::default(),
			scripts: Default::default(),
			dependencies: Default::default(),
			sources: Default::default(),
		}
	}

	#[must_use]
	pub fn from_resolved(from_registry: &Url, resolved: ResolvedManifest) -> Self {
		let dependencies = resolved
			.dependencies
			.into_inner()
			.into_iter()
			.map(|(alias, (spec, ty))| {
				let spec = match spec {
					ResolvedDependencySpecifier::Pesde {
						name,
						req,
						registry,
					} => DependencySpecifier::Pesde(PesdeDependencySpecifier {
						name,
						req: req.into_inner(),
						registry: registry
							.map_or_else(|| from_registry.clone(), BoundedUrl::into_inner),
					}),
					ResolvedDependencySpecifier::Wally { name, req, index } => {
						DependencySpecifier::Wally(WallyDependencySpecifier {
							name,
							req: req.into_inner(),
							index: index.into_inner(),
						})
					}
				};

				(alias, (spec, ty))
			})
			.collect::<BTreeMap<_, _>>();

		Self {
			package: resolved.package,
			workspace: ManifestWorkspace {
				members: Default::default(),
				overrides: Default::default(),
			},
			publish: false,
			includes: Default::default(),
			exports: Some(resolved.exports),
			config: resolved.config_luau,
			hooks: Default::default(),
			scripts: Default::default(),
			// the same const limit, cannot map to a different size so this'll never fail
			dependencies: BoundedBTreeMap::new(dependencies).unwrap(),
			sources: Default::default(),
		}
	}
}
