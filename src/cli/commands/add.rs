#![expect(deprecated)]
use std::str::FromStr as _;

use anyhow::Context as _;
use clap::Args;
use pesde::source::Realm;
use pesde::source::pesde::target::TargetKind;
use semver::VersionReq;

use crate::cli::AnyPackageIdentifier;
use crate::cli::VersionedPackageName;
use crate::cli::config::read_config;
use crate::cli::dep_type_to_key;
use pesde::DEFAULT_INDEX_NAME;
use pesde::RefreshedSources;
use pesde::Subproject;
use pesde::manifest::Alias;
use pesde::manifest::DependencyType;
use pesde::names::PackageNames;
use pesde::source::DependencySpecifiers;
use pesde::source::PackageSources;
use pesde::source::git::GitPackageSource;
use pesde::source::git::specifier::GitDependencySpecifier;
use pesde::source::git::specifier::GitVersionSpecifier;
use pesde::source::path::PathPackageSource;
use pesde::source::path::RelativeOrAbsolutePath;
use pesde::source::path::specifier::PathDependencySpecifier;
use pesde::source::pesde::PesdePackageSource;
use pesde::source::pesde::specifier::PesdeDependencySpecifier;
use pesde::source::traits::PackageSource as _;
use pesde::source::traits::RefreshOptions;
use pesde::source::traits::ResolveOptions;

#[derive(Debug, Args)]
pub struct AddCommand {
	/// The package name to add
	#[arg(index = 1)]
	name: AnyPackageIdentifier<VersionReq>,

	/// The index in which to search for the package
	#[arg(short, long)]
	index: Option<String>,

	/// The realm for the package
	#[arg(short, long)]
	realm: Option<Realm>,

	/// The alias to use for the package
	#[arg(short, long)]
	alias: Option<Alias>,

	/// Whether to add the package as a peer dependency
	#[arg(short, long)]
	peer: bool,

	/// Whether to add the package as a dev dependency
	#[arg(short, long, conflicts_with = "peer")]
	dev: bool,
}

impl AddCommand {
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		let manifest = subproject
			.deser_manifest()
			.await
			.context("failed to read manifest")?;

		let (source, specifier) = match &self.name {
			AnyPackageIdentifier::PackageName(versioned) => match &versioned {
				#[expect(deprecated)]
				VersionedPackageName(PackageNames::Pesde(name), version) => {
					let index = manifest
						.indices
						.pesde
						.get(self.index.as_deref().unwrap_or(DEFAULT_INDEX_NAME))
						.cloned();

					if let Some(index) = self.index.as_ref().filter(|_| index.is_none()) {
						anyhow::bail!("index {index} not found");
					}

					let index = match index {
						Some(index) => index,
						None => read_config().await?.default_index,
					};

					let source = PackageSources::Pesde(PesdePackageSource::new(index));
					let specifier = DependencySpecifiers::Pesde(PesdeDependencySpecifier {
						name: name.clone(),
						version: version.clone().unwrap_or(VersionReq::STAR),
						index: self.index.unwrap_or_else(|| DEFAULT_INDEX_NAME.to_string()),
						target: match self.realm {
							Some(Realm::Shared) => TargetKind::Roblox,
							Some(Realm::Server) => TargetKind::RobloxServer,
							None => TargetKind::Luau,
						},
					});

					(source, specifier)
				}
				VersionedPackageName(PackageNames::Wally(name), version) => {
					let index = manifest
						.indices
						.wally
						.get(self.index.as_deref().unwrap_or(DEFAULT_INDEX_NAME))
						.cloned();

					if let Some(index) = self.index.as_ref().filter(|_| index.is_none()) {
						anyhow::bail!("wally index {index} not found");
					}

					let index = index.context("no wally index found")?;

					let source =
						PackageSources::Wally(pesde::source::wally::WallyPackageSource::new(index));
					let specifier = DependencySpecifiers::Wally(
						pesde::source::wally::specifier::WallyDependencySpecifier {
							name: name.clone(),
							version: version.clone().unwrap_or(VersionReq::STAR),
							index: self.index.unwrap_or_else(|| DEFAULT_INDEX_NAME.to_string()),
							realm: self.realm.context("wally packages require a realm")?,
						},
					);

					(source, specifier)
				}
			},
			AnyPackageIdentifier::Git((url, ver)) => (
				PackageSources::Git(GitPackageSource::new(url.clone())),
				DependencySpecifiers::Git(GitDependencySpecifier {
					repo: url.clone(),
					version_specifier: ver.clone(),
					path: None,
					realm: self.realm,
				}),
			),
			AnyPackageIdentifier::Path(path) => (
				PackageSources::Path(PathPackageSource),
				DependencySpecifiers::Path(PathDependencySpecifier {
					path: path.clone(),
					realm: self.realm,
				}),
			),
		};

		let refreshed_sources = RefreshedSources::new();

		refreshed_sources
			.refresh(
				&source,
				&RefreshOptions {
					project: subproject.project().clone(),
				},
			)
			.await
			.context("failed to refresh package source")?;

		let (_, _, mut versions) = source
			.resolve(
				&specifier,
				&ResolveOptions {
					subproject: subproject.clone(),
					refreshed_sources,
				},
			)
			.await
			.context("failed to resolve package")?;

		let Some((version, _)) = versions.pop_last() else {
			anyhow::bail!("no matching versions found for package");
		};

		let mut manifest = toml_edit::DocumentMut::from_str(
			&subproject
				.read_manifest()
				.await
				.context("failed to read manifest")?,
		)
		.context("failed to parse manifest")?;
		let dependency_key = dep_type_to_key(if self.peer {
			DependencyType::Peer
		} else if self.dev {
			DependencyType::Dev
		} else {
			DependencyType::Standard
		});

		let alias = match self.alias {
			Some(alias) => alias,
			None => match &self.name {
				AnyPackageIdentifier::PackageName(versioned) => versioned.0.name().to_string(),
				AnyPackageIdentifier::Git((url, _)) => url
					.as_url()
					.path
					.to_string()
					.split('/')
					.next_back()
					.map_or_else(|| url.as_url().path.to_string(), ToString::to_string),
				AnyPackageIdentifier::Path(path) => match path {
					RelativeOrAbsolutePath::Relative(path) => {
						path.file_name().map(ToString::to_string)
					}
					RelativeOrAbsolutePath::Absolute(path) => {
						path.file_name().map(|s| s.to_string_lossy().to_string())
					}
				}
				.expect("path has no file name"),
			}
			.parse()
			.context("auto-generated alias is invalid. use --alias to specify one")?,
		};

		let field = &mut manifest[dependency_key]
			.or_insert(toml_edit::Item::Table(toml_edit::Table::new()))[alias.as_str()];

		match specifier {
			#[expect(deprecated)]
			DependencySpecifiers::Pesde(spec) => {
				field["name"] = toml_edit::value(spec.name.to_string());
				field["version"] = toml_edit::value(format!("^{version}"));

				field["target"] = toml_edit::value(spec.target.to_string());

				if spec.index != DEFAULT_INDEX_NAME {
					field["index"] = toml_edit::value(spec.index);
				}

				println!(
					"added {}@{version} {} to {dependency_key}",
					spec.name, spec.target
				);
			}
			DependencySpecifiers::Wally(spec) => {
				let name_str = spec.name.to_string();
				let name_str = name_str.trim_start_matches("wally#");
				field["wally"] = toml_edit::value(name_str);
				field["version"] = toml_edit::value(format!("^{version}"));

				if spec.index != DEFAULT_INDEX_NAME {
					field["index"] = toml_edit::value(spec.index);
				}

				println!("added wally {name_str}@{version} to {dependency_key}");
			}
			DependencySpecifiers::Git(spec) => {
				field["repo"] = toml_edit::value(spec.repo.to_string());
				match spec.version_specifier.clone() {
					GitVersionSpecifier::Rev(rev) => field["rev"] = toml_edit::value(rev),
					GitVersionSpecifier::VersionReq(req) => {
						field["version"] = toml_edit::value(req.to_string());
					}
				}

				println!(
					"added git {}{} to {dependency_key}",
					spec.repo, spec.version_specifier
				);
			}
			DependencySpecifiers::Path(spec) => {
				field["path"] = toml_edit::value(spec.path.to_string());

				println!("added path {} to {dependency_key}", spec.path);
			}
		}

		subproject
			.write_manifest(manifest.to_string())
			.await
			.context("failed to write manifest")?;

		Ok(())
	}
}
