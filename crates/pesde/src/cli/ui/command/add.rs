use std::str::FromStr as _;
use std::sync::Arc;

use anyhow::Context as _;
use clap::Args;
use futures::TryStreamExt as _;
use pesde::registry::manifest::Alias;
use pesde::registry::manifest::DependencyType;
use pesde::source::DependencySpecifier;
use pesde::source::PackageId;
use pesde::source::PackageIdComparison;

use crate::cli::ui::CommandContext;
use crate::cli::ui::dependency_type_to_key;
use crate::cli::ui::specifier::CliDependencySpecifier;

/// Adds a package to the manifest
#[derive(Debug, Args)]
pub struct AddCommand {
	/// The package to add
	package: CliDependencySpecifier,

	/// The alias to use for the package
	#[arg(short, long)]
	alias: Option<Alias>,

	/// The name to use for the source. Only needed when passing in the unnamed form
	#[arg(short, long)]
	source: Option<Arc<str>>,

	/// Whether to add the package as a peer dependency
	#[arg(short, long)]
	peer: bool,

	/// Whether to add the package as a dev dependency
	#[arg(short, long, conflicts_with = "peer")]
	dev: bool,
}

impl AddCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let project = ctx.project().await?;
		let member = ctx.member().await?;

		let source_name = match &self.package {
			CliDependencySpecifier::Full(..) => self
				.source
				.context("source name is required when passing in the full form")?,
			CliDependencySpecifier::Named(source, _) => source.clone(),
		};

		let specifier = self.package.resolve(member).await?;

		let alias = match self.alias {
			Some(alias) => alias,
			None => match &specifier {
				DependencySpecifier::Pesde(specifier) => specifier.name.local_name().as_str(),
				DependencySpecifier::Wally(specifier) => specifier.name.local_name().as_str(),
				#[expect(deprecated)]
				DependencySpecifier::LegacyPesde(specifier) => specifier.name.local_name().as_str(),
				DependencySpecifier::Git(specifier) => specifier
					.url
					.path()
					.split('/')
					.next_back()
					.unwrap_or(specifier.url.path()),
				DependencySpecifier::Workspace(specifier) => specifier.name.local_name().as_str(),
			}
			.parse()
			.context("auto-generated alias is invalid. use --alias to specify one")?,
		};

		let mut manifest = toml_edit::DocumentMut::from_str(
			&member
				.read_manifest()
				.await
				.context("failed to read manifest")?,
		)
		.context("failed to parse manifest")?;
		let dependency_key = dependency_type_to_key(if self.peer {
			DependencyType::Peer
		} else if self.dev {
			DependencyType::Dev
		} else {
			DependencyType::Standard
		});

		let dependencies =
			manifest[dependency_key].or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
		let Some(dependencies) = dependencies.as_table_like_mut() else {
			anyhow::bail!("invalid manifest: `{dependency_key}` is not a table");
		};
		let dependency_field = match dependencies.entry(&alias) {
			toml_edit::Entry::Occupied(_) => {
				anyhow::bail!("package `{alias}` already exists in `{dependency_key}`");
			}
			toml_edit::Entry::Vacant(entry) => entry,
		};

		let versions = crate::cli::ui::specifier::resolve(project, specifier.clone())
			.await
			.context("failed to resolve package")?;

		let Some(chosen_package) = versions
			.try_fold(None::<PackageId>, async |acc, (candidate, _)| {
				let Some(acc) = acc else {
					return Ok(Some(candidate));
				};

				match candidate.compare(&acc) {
					PackageIdComparison::Comparable(ordering) => {
						if ordering == std::cmp::Ordering::Greater {
							Ok(Some(candidate))
						} else {
							Ok(Some(acc))
						}
					}
					PackageIdComparison::Incomparable => {
						// should only ever be one anyway
						Ok(Some(acc))
					}
					PackageIdComparison::NotSameSource => anyhow::bail!(
						"resolved package `{candidate}` is not of the same source as previously resolved package `{acc}`"
					),
				}
			})
			.await
			.context("failed to find version")?
		else {
			return Err(anyhow::format_err!(
				"no versions found for package `{specifier}`"
			));
		};

		let source_prefix = if &*source_name == "default" {
			format_args!("")
		} else {
			format_args!("{source_name}:")
		};

		let insert_source = |manifest: &mut toml_edit::DocumentMut, value: toml_edit::Item| {
			let sources =
				manifest["sources"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
			let Some(sources) = sources.as_table_like_mut() else {
				anyhow::bail!("invalid manifest: `sources` is not a table");
			};
			sources.insert(&source_name, value);
			Ok(())
		};

		match specifier {
			DependencySpecifier::Pesde(_) => {
				let PackageId::Pesde(url, name, version) = &chosen_package else {
					unreachable!("resolved package is not a pesde package");
				};

				let string = format!("{source_prefix}{name}@^{version}");
				println!("added {string} to {dependency_key}");

				dependency_field.insert(string.into());
				insert_source(&mut manifest, url.to_string().into()).unwrap();
			}
			DependencySpecifier::LegacyPesde(_) => {
				let PackageId::LegacyPesde(url, name, version, target) = &chosen_package else {
					unreachable!("resolved package is not a legacy pesde package");
				};

				let string = format!("{source_prefix}{name}@^{version} {target}");
				println!("added {string} to {dependency_key}");

				dependency_field.insert(string.into());
				insert_source(
					&mut manifest,
					toml_edit::Value::from_iter([
						(toml_edit::Key::new("kind"), "legacy_pesde"),
						(toml_edit::Key::new("url"), &url.to_string()),
					])
					.into(),
				)?;
			}
			DependencySpecifier::Wally(_) => {
				let PackageId::Wally(url, name, version) = &chosen_package else {
					unreachable!("resolved package is not a wally package");
				};

				let string = format!("{source_prefix}{name}@^{version}");
				println!("added {string} to {dependency_key}");

				dependency_field.insert(string.into());
				insert_source(
					&mut manifest,
					toml_edit::Value::from_iter([
						(toml_edit::Key::new("kind"), "wally"),
						(toml_edit::Key::new("url"), &url.to_string()),
					])
					.into(),
				)?;
			}
			DependencySpecifier::Git(_) => {
				let PackageId::Git(url, revision, path) = &chosen_package else {
					unreachable!("resolved package is not a git package");
				};

				let path = if path.is_empty() {
					format_args!("")
				} else {
					format_args!(":{path}")
				};

				let string = format!("{source_prefix}{path}");
				println!("added {string} to {dependency_key}");

				dependency_field.insert(string.into());
				insert_source(
					&mut manifest,
					toml_edit::Value::from_iter([
						(toml_edit::Key::new("kind"), "git"),
						(toml_edit::Key::new("url"), &url.to_string()),
						(toml_edit::Key::new("rev"), revision),
					])
					.into(),
				)?;
			}
			DependencySpecifier::Workspace(spec) => {
				let string = format!("{source_prefix}:{}", spec.name);
				println!("added {string} to {dependency_key}");

				dependency_field.insert(string.into());
			}
		}

		member
			.write_manifest(manifest.to_string())
			.await
			.context("failed to write manifest")?;

		Ok(())
	}
}
