use std::collections::BTreeMap;

use crate::{
	manifest::{Alias, DependencyType, errors, target::TargetKind},
	source::{specifiers::DependencySpecifiers, wally::specifier::WallyDependencySpecifier},
};
use semver::{Version, VersionReq};
use serde::{Deserialize, Deserializer};
use tracing::instrument;

#[derive(Deserialize, Copy, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Realm {
	#[serde(alias = "dev")]
	Shared,
	Server,
}

impl Realm {
	pub fn to_target(self) -> TargetKind {
		match self {
			Realm::Shared => TargetKind::Roblox,
			Realm::Server => TargetKind::RobloxServer,
		}
	}
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct WallyPackage {
	pub version: Version,
	pub registry: url::Url,
	pub realm: Realm,
}

pub fn deserialize_specifiers<'de, D: Deserializer<'de>>(
	deserializer: D,
) -> Result<BTreeMap<Alias, WallyDependencySpecifier>, D::Error> {
	// specifier is in form of `name@version_req`
	BTreeMap::<Alias, String>::deserialize(deserializer)?
		.into_iter()
		.map(|(k, v)| {
			let (name, version) = v.split_once('@').ok_or_else(|| {
				serde::de::Error::custom("invalid specifier format, expected `name@version_req`")
			})?;

			Ok((
				k,
				WallyDependencySpecifier {
					name: name.parse().map_err(serde::de::Error::custom)?,
					version: VersionReq::parse(version).map_err(serde::de::Error::custom)?,
					// doesn't matter, will be replaced later
					index: "".to_string(),
				},
			))
		})
		.collect()
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct WallyManifest {
	pub package: WallyPackage,
	#[serde(default, deserialize_with = "deserialize_specifiers")]
	dependencies: BTreeMap<Alias, WallyDependencySpecifier>,
	#[serde(default, deserialize_with = "deserialize_specifiers")]
	server_dependencies: BTreeMap<Alias, WallyDependencySpecifier>,
	#[serde(default, deserialize_with = "deserialize_specifiers")]
	dev_dependencies: BTreeMap<Alias, WallyDependencySpecifier>,
}

impl WallyManifest {
	/// Get all dependencies from the manifest
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub fn all_dependencies(
		&mut self,
	) -> Result<BTreeMap<Alias, (DependencySpecifiers, DependencyType)>, errors::AllDependenciesError>
	{
		let mut all_deps = BTreeMap::new();

		for (deps, ty) in [
			(&mut self.dependencies, DependencyType::Standard),
			(&mut self.server_dependencies, DependencyType::Standard),
			(&mut self.dev_dependencies, DependencyType::Dev),
		] {
			while let Some((alias, mut spec)) = deps.pop_first() {
				spec.index = self.package.registry.to_string();

				if all_deps
					.insert(alias.clone(), (DependencySpecifiers::Wally(spec), ty))
					.is_some()
				{
					return Err(errors::AllDependenciesError::AliasConflict(alias));
				}
			}
		}

		Ok(all_deps)
	}
}
