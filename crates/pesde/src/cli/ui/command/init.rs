use clap::Args;
use inquire::validator::Validation;
use pesde::Url;
use pesde::registry::PackageName;
use std::str::FromStr as _;

use crate::cli::ui::CommandContext;

/// Initializes a manifest file in the current directory
#[derive(Debug, Args)]
pub struct InitCommand;

impl InitCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let cli_config = ctx.cli_config().await?;
		let member = ctx.member().await?;
		match member.read_manifest().await {
			Ok(_) => {
				anyhow::bail!("project already initialized");
			}
			Err(e)
				if let pesde::ErrorKind::Vfs(_, e) = e.inner()
					&& e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(e.into()),
		}

		let mut manifest = toml_edit::DocumentMut::new();
		let package =
			manifest["package"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()));

		package["name"] = toml_edit::value(
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
		package["version"] = toml_edit::value("0.1.0");

		let description = inquire::Text::new("what is the description of the project?")
			.with_help_message("a short description of the project. leave empty for none")
			.prompt()
			.unwrap();

		if !description.is_empty() {
			package["description"] = toml_edit::value(description);
		}

		let repo = inquire::Text::new("what is the repository URL of this project?")
			.with_validator(|repo: &str| {
				if repo.is_empty() {
					return Ok(Validation::Valid);
				}

				Ok(match Url::from_str(repo) {
					Ok(_) => Validation::Valid,
					Err(e) => Validation::Invalid(e.to_string().into()),
				})
			})
			.with_help_message("leave empty for none")
			.prompt()
			.unwrap();
		if !repo.is_empty() {
			package["repository"] = toml_edit::value(repo);
		}

		let license = inquire::Text::new("what is the license of this project?")
			.with_initial_value("MIT")
			.with_help_message("an SPDX license identifier. leave empty for none")
			.prompt()
			.unwrap();
		if !license.is_empty() {
			package["license"] = toml_edit::value(license);
		}

		manifest["sources"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()))["default"] =
			toml_edit::value(cli_config.default_registry.to_string());

		member.write_manifest(manifest.to_string()).await?;

		println!("initialized project! run `install` to fully finish setup");
		Ok(())
	}
}
