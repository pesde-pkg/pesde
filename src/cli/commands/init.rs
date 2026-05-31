use clap::Args;
use inquire::validator::Validation;
use pesde::DEFAULT_URL_KEY;
use pesde::Subproject;
use pesde::Url;
use pesde::errors::ManifestReadErrorKind;
use pesde::names::PackageName;
use std::str::FromStr as _;

use crate::cli::config::read_config;
use crate::cli::style::INFO_STYLE;
use crate::cli::style::SUCCESS_STYLE;

#[derive(Debug, Args)]
pub struct InitCommand;

impl InitCommand {
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		match subproject.read_manifest().await {
			Ok(_) => {
				anyhow::bail!("project already initialized");
			}
			Err(e)
				if let ManifestReadErrorKind::Io(e) = e.inner()
					&& e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(e.into()),
		}

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

				Ok(match Url::from_str(repo) {
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

		manifest["registries"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
			[DEFAULT_URL_KEY] = toml_edit::value(read_config().await?.default_registry.to_string());

		subproject.write_manifest(manifest.to_string()).await?;

		println!(
			"{}\n{}: run `install` to fully finish setup",
			SUCCESS_STYLE.apply_to("initialized project"),
			INFO_STYLE.apply_to("tip")
		);
		Ok(())
	}
}
