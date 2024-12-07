use crate::cli::config::read_config;
use anyhow::Context;
use clap::Args;
use colored::Colorize;
use inquire::validator::Validation;
use pesde::{
    errors::ManifestReadError,
    manifest::target::TargetKind,
    names::PackageName,
    source::{
        git_index::GitBasedSource,
        pesde::{specifier::PesdeDependencySpecifier, PesdePackageSource},
        traits::PackageSource,
    },
    Project, DEFAULT_INDEX_NAME, SCRIPTS_LINK_FOLDER,
};
use semver::VersionReq;
use std::{collections::HashSet, str::FromStr};

#[derive(Debug, Args)]
pub struct InitCommand {}

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

        if target_env.is_roblox()
            || inquire::prompt_confirmation(
                "would you like to setup default Roblox compatibility scripts?",
            )
            .unwrap()
        {
            PackageSource::refresh(&source, &project)
                .await
                .context("failed to refresh package source")?;
            let config = source
                .config(&project)
                .await
                .context("failed to get source config")?;

            if let Some(scripts_pkg_name) = config.scripts_package {
                let (v_id, pkg_ref) = source
                    .resolve(
                        &PesdeDependencySpecifier {
                            name: scripts_pkg_name,
                            version: VersionReq::STAR,
                            index: None,
                            target: None,
                        },
                        &project,
                        TargetKind::Lune,
                        &mut HashSet::new(),
                    )
                    .await
                    .context("failed to resolve scripts package")?
                    .1
                    .pop_last()
                    .context("scripts package not found")?;

                let Some(scripts) = pkg_ref.target.scripts().filter(|s| !s.is_empty()) else {
                    anyhow::bail!("scripts package has no scripts. this is an issue with the index")
                };

                let scripts_field = &mut manifest["scripts"]
                    .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));

                for script_name in scripts.keys() {
                    scripts_field[script_name] = toml_edit::value(format!(
                        "{SCRIPTS_LINK_FOLDER}/scripts/{script_name}.luau"
                    ));
                }

                let field = &mut manifest["dev_dependencies"]
                    .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))["scripts"];
                field["name"] = toml_edit::value(pkg_ref.name.to_string());
                field["version"] = toml_edit::value(format!("^{}", v_id.version()));
                field["target"] = toml_edit::value(v_id.target().to_string());
            } else {
                println!(
                    "{}",
                    "configured index hasn't a configured scripts package".red()
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
