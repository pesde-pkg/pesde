use std::collections::BTreeMap;

use crate::manifest::Alias;
use crate::manifest::DependencyType;
use crate::manifest::errors;
use crate::names::wally::WallyPackageName;
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
	pub name: WallyPackageName,
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
					// replaced in into_resolve_entry based on the package's registry field
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
	WallyPackage,
	BTreeMap<Alias, (DependencySpecifiers, DependencyType)>,
);

impl WallyManifest {
	#[instrument(skip(self), ret(level = "trace"), level = "debug")]
	pub(crate) fn into_resolve_entry(self) -> Result<ResolveEntry, errors::AllDependenciesError> {
		let mut all_deps = BTreeMap::new();

		for (mut deps, ty) in [
			(self.dependencies, DependencyType::Standard),
			(self.server_dependencies, DependencyType::Standard),
			(self.dev_dependencies, DependencyType::Dev),
		] {
			while let Some((alias, mut spec)) = deps.pop_first() {
				spec.index = self.package.registry.to_string();

				// TODO: update realm based on the package's canonical value (WallyPackage.realm)

				if all_deps
					.insert(alias.clone(), (DependencySpecifiers::Wally(spec), ty))
					.is_some()
				{
					return Err(errors::AllDependenciesErrorKind::AliasConflict(alias).into());
				}
			}
		}

		Ok((self.package, all_deps))
	}
}
