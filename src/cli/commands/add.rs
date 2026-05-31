use std::str::FromStr as _;

use anyhow::Context as _;
use clap::Args;
use pesde::source::Realm;
use pesde::source::ResolveResult;

use crate::cli::AnyPackageIdentifier;
use crate::cli::dep_type_to_key;
use crate::cli::install::get_lockfile;
use pesde::DEFAULT_URL_KEY;
use pesde::RefreshedSources;
use pesde::Subproject;
use pesde::manifest::Alias;
use pesde::manifest::DependencyType;
use pesde::source::DependencySpecifiers;
use pesde::source::PackageSource as _;
use pesde::source::path::RelativeOrAbsolutePath;

#[derive(Debug, Args)]
pub struct AddCommand {
	/// The package to add
	#[arg(index = 1)]
	package: AnyPackageIdentifier,

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
		let (source, specifier) = self
			.package
			.source_and_specifier(self.realm, async |pesde| {
				let manifest = subproject
					.deser_manifest()
					.await
					.context("failed to read manifest")?;

				let indices = if pesde {
					&manifest.pesde_indices
				} else {
					&manifest.wally_indices
				};

				let name = self.index.as_deref().unwrap_or(DEFAULT_URL_KEY);
				Ok((
					name.to_string(),
					indices
						.get(name)
						.with_context(|| format!("index `{name}` not found"))?
						.clone(),
				))
			})
			.await
			.context("failed to parse package identifier")?;

		let refreshed_sources = RefreshedSources::new();

		let lockfile = get_lockfile(subproject.project(), &refreshed_sources).await?;

		let source_state = refreshed_sources
			.refresh(
				&source,
				subproject.project(),
				lockfile.source_states.get(&source),
			)
			.await
			.context("failed to refresh package source")?;

		let ResolveResult { mut versions, .. } = source
			.resolve(&subproject, &source_state, &specifier, &refreshed_sources)
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
			None => match &self.package {
				AnyPackageIdentifier::PesdePackageName(versioned) => versioned.0.name().as_str(),
				AnyPackageIdentifier::WallyPackageName(versioned) => versioned.0.name(),
				AnyPackageIdentifier::Git((url, _)) => {
					url.path().split('/').next_back().unwrap_or(url.path())
				}
				AnyPackageIdentifier::Path(path) => match path {
					RelativeOrAbsolutePath::Relative(path) => path.file_name(),
					RelativeOrAbsolutePath::Absolute(path) => {
						path.file_name().and_then(|s| s.to_str())
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
			DependencySpecifiers::Pesde(spec) => {
				field["name"] = toml_edit::value(spec.name.to_string());
				field["version"] = toml_edit::value(format!("^{version}"));

				if spec.registry != DEFAULT_URL_KEY {
					field["registry"] = toml_edit::value(spec.registry);
				}

				println!("added {}@{version} to {dependency_key}", spec.name);
			}
			#[expect(deprecated)]
			DependencySpecifiers::LegacyPesde(spec) => {
				field["name"] = toml_edit::value(spec.name.to_string());
				field["version"] = toml_edit::value(format!("^{version}"));

				field["target"] = toml_edit::value(spec.target.to_string());

				if spec.index != DEFAULT_URL_KEY {
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

				if spec.index != DEFAULT_URL_KEY {
					field["index"] = toml_edit::value(spec.index);
				}

				println!("added wally {name_str}@{version} to {dependency_key}");
			}
			DependencySpecifiers::Git(spec) => {
				field["repo"] = toml_edit::value(spec.repo.to_string());
				field["rev"] = toml_edit::value(&spec.rev);

				println!("added git {}#{} to {dependency_key}", spec.repo, spec.rev);
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
