use std::collections::BTreeMap;

use crate::manifest::Alias;
use crate::manifest::DependencyType;
use crate::manifest::errors;
use crate::source::DependencySpecifiers;
use crate::source::wally::specifier::WallyDependencySpecifier;
use semver::Version;
use semver::VersionReq;
use serde::Deserialize;
use serde::Deserializer;
use tracing::instrument;

#[derive(Deserialize, Copy, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum WallyRealm {
	#[serde(alias = "dev")]
	Shared,
	Server,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct WallyPackage {
	pub version: Version,
	pub registry: url::Url,
	#[allow(unused)]
	pub realm: WallyRealm,
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
					// Wally realms function differently to pesde's, so shared is the most reasonable default
					realm: crate::source::Realm::Shared,
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

type ResolveEntry = (
	Version,
	BTreeMap<Alias, (DependencySpecifiers, DependencyType)>,
);

impl WallyManifest {
	/// Get all dependencies from the manifest
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub fn into_resolve_entry(self) -> Result<ResolveEntry, errors::AllDependenciesError> {
		let mut all_deps = BTreeMap::new();

		for (mut deps, ty) in [
			(self.dependencies, DependencyType::Standard),
			(self.server_dependencies, DependencyType::Standard),
			(self.dev_dependencies, DependencyType::Dev),
		] {
			while let Some((alias, mut spec)) = deps.pop_first() {
				spec.index = self.package.registry.to_string();

				if all_deps
					.insert(alias.clone(), (DependencySpecifiers::Wally(spec), ty))
					.is_some()
				{
					return Err(errors::AllDependenciesErrorKind::AliasConflict(alias).into());
				}
			}
		}

		Ok((self.package.version, all_deps))
	}
}
