use crate::cli::config::read_config;
use anyhow::Context;
use clap::Args;
use colored::Colorize;
use inquire::validator::Validation;
use pesde::{
	errors::ManifestReadError,
	manifest::{target::TargetKind, DependencyType},
	names::{PackageName, PackageNames},
	source::{
		git_index::GitBasedSource,
		ids::PackageId,
		pesde::{specifier::PesdeDependencySpecifier, PesdePackageSource},
		specifiers::DependencySpecifiers,
		traits::{GetTargetOptions, PackageSource, RefreshOptions, ResolveOptions},
		PackageSources,
	},
	Project, RefreshedSources, DEFAULT_INDEX_NAME, SCRIPTS_LINK_FOLDER,
};
use semver::VersionReq;
use std::{fmt::Display, path::Path, str::FromStr, sync::Arc};

#[derive(Debug, Args)]
pub struct InitCommand {}

#[derive(Debug)]
enum PackageNameOrCustom {
	PackageName(PackageName),
	Custom,
}

impl Display for PackageNameOrCustom {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			PackageNameOrCustom::PackageName(n) => write!(f, "{n}"),
			PackageNameOrCustom::Custom => write!(f, "custom"),
		}
	}
}

impl InitCommand {
	pub async fn run(self, project: Project) -> anyhow::Result<()> {
		match project.read_manifest().await {
			Ok(_) => {
				println!("{}", "project already initialized".red());
				return Ok(());
			}
			Err(ManifestReadError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(e.into()),
		};

		let mut manifest = toml_edit::DocumentMut::new();

		manifest["name"] = toml_edit::value(
			inquire::Text::new("what is the name of the project?")
				.with_validator(|name: &str| {
					Ok(match PackageName::from_str(name) {
						Ok(_) => Validation::Valid,
						Err(e) => Validation::Invalid(e.to_string().into()),
					})
				})
				.prompt()
				.unwrap(),
		);
		manifest["version"] = toml_edit::value("0.1.0");

		let description = inquire::Text::new("what is the description of the project?")
			.with_help_message("a short description of the project. leave empty for none")
			.prompt()
			.unwrap();

		if !description.is_empty() {
			manifest["description"] = toml_edit::value(description);
		}

		let authors = inquire::Text::new("who are the authors of this project?")
			.with_help_message("comma separated list. leave empty for none")
			.prompt()
			.unwrap();

		let authors = authors
			.split(',')
			.map(str::trim)
			.filter(|s| !s.is_empty())
			.collect::<toml_edit::Array>();

		if !authors.is_empty() {
			manifest["authors"] = toml_edit::value(authors);
		}

		let repo = inquire::Text::new("what is the repository URL of this project?")
			.with_validator(|repo: &str| {
				if repo.is_empty() {
					return Ok(Validation::Valid);
				}

				Ok(match url::Url::parse(repo) {
					Ok(_) => Validation::Valid,
					Err(e) => Validation::Invalid(e.to_string().into()),
				})
			})
			.with_help_message("leave empty for none")
			.prompt()
			.unwrap();
		if !repo.is_empty() {
			manifest["repository"] = toml_edit::value(repo);
		}

		let license = inquire::Text::new("what is the license of this project?")
			.with_initial_value("MIT")
			.with_help_message("an SPDX license identifier. leave empty for none")
			.prompt()
			.unwrap();
		if !license.is_empty() {
			manifest["license"] = toml_edit::value(license);
		}

		let target_env = inquire::Select::new(
			"what environment are you targeting for your package?",
			TargetKind::VARIANTS.to_vec(),
		)
		.prompt()
		.unwrap();

		manifest["target"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
			["environment"] = toml_edit::value(target_env.to_string());

		let source = PesdePackageSource::new(read_config().await?.default_index);

		manifest["indices"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
			[DEFAULT_INDEX_NAME] = toml_edit::value(source.repo_url().to_bstring().to_string());

		let refreshed_sources = RefreshedSources::new();

		if target_env.is_roblox()
			|| inquire::prompt_confirmation(
				"would you like to setup default Roblox compatibility scripts?",
			)
			.unwrap()
		{
			refreshed_sources
				.refresh(
					&PackageSources::Pesde(source.clone()),
					&RefreshOptions {
						project: project.clone(),
					},
				)
				.await
				.context("failed to refresh package source")?;
			let config = source
				.config(&project)
				.await
				.context("failed to get source config")?;

			let scripts_package = if config.scripts_packages.is_empty() {
				PackageNameOrCustom::Custom
			} else {
				inquire::Select::new(
					"which scripts package do you want to use?",
					config
						.scripts_packages
						.into_iter()
						.map(PackageNameOrCustom::PackageName)
						.chain(std::iter::once(PackageNameOrCustom::Custom))
						.collect(),
				)
				.prompt()
				.unwrap()
			};

			let scripts_package = match scripts_package {
				PackageNameOrCustom::PackageName(p) => Some(p),
				PackageNameOrCustom::Custom => {
					let name = inquire::Text::new("which scripts package to use?")
						.with_validator(|name: &str| {
							if name.is_empty() {
								return Ok(Validation::Valid);
							}

							Ok(match PackageName::from_str(name) {
								Ok(_) => Validation::Valid,
								Err(e) => Validation::Invalid(e.to_string().into()),
							})
						})
						.with_help_message("leave empty for none")
						.prompt()
						.unwrap();

					if name.is_empty() {
						None
					} else {
						Some(PackageName::from_str(&name).unwrap())
					}
				}
			};

			if let Some(scripts_pkg_name) = scripts_package {
				let (v_id, pkg_ref) = source
					.resolve(
						&PesdeDependencySpecifier {
							name: scripts_pkg_name.clone(),
							version: VersionReq::STAR,
							index: None,
							target: None,
						},
						&ResolveOptions {
							project: project.clone(),
							target: TargetKind::Lune,
							refreshed_sources,
						},
					)
					.await
					.context("failed to resolve scripts package")?
					.1
					.pop_last()
					.context("scripts package not found")?;

				let id = Arc::new(PackageId::new(PackageNames::Pesde(scripts_pkg_name), v_id));

				let target = source
					.get_target(
						&pkg_ref,
						&GetTargetOptions {
							project: project.clone(),
							// HACK: the pesde package source doesn't use the path, so we can just use an empty one
							path: Arc::from(Path::new("")),
							id: id.clone(),
						},
					)
					.await?;

				let Some(scripts) = target.scripts().filter(|s| !s.is_empty()) else {
					anyhow::bail!("scripts package has no scripts.")
				};

				let scripts_field = &mut manifest["scripts"]
					.or_insert(toml_edit::Item::Table(toml_edit::Table::new()));

				for script_name in scripts.keys() {
					scripts_field[script_name] = toml_edit::value(format!(
						"{SCRIPTS_LINK_FOLDER}/scripts/{script_name}.luau"
					));
				}

				let dev_deps = &mut manifest["dev_dependencies"]
					.or_insert(toml_edit::Item::Table(toml_edit::Table::new()));

				let field = &mut dev_deps["scripts"];
				field["name"] = toml_edit::value(id.name().to_string());
				field["version"] = toml_edit::value(format!("^{}", id.version_id().version()));
				field["target"] = toml_edit::value(id.version_id().target().to_string());

				for (alias, (spec, ty)) in pkg_ref.dependencies {
					if ty != DependencyType::Peer {
						continue;
					}

					let DependencySpecifiers::Pesde(spec) = spec else {
						continue;
					};

					let field = &mut dev_deps[alias.as_str()];
					field["name"] = toml_edit::value(spec.name.to_string());
					field["version"] = toml_edit::value(spec.version.to_string());
					field["target"] = toml_edit::value(
						spec.target
							.unwrap_or_else(|| *id.version_id().target())
							.to_string(),
					);
				}
			} else {
				println!(
                    "{}",
                    "no scripts package configured, this can cause issues with Roblox compatibility".red()
                );
				if !inquire::prompt_confirmation("initialize regardless?").unwrap() {
					return Ok(());
				}
			}
		}

		project.write_manifest(manifest.to_string()).await?;

		println!(
			"{}\n{}: run `install` to fully finish setup",
			"initialized project".green(),
			"tip".cyan().bold()
		);
		Ok(())
	}
}
