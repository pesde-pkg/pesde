use std::{path::Path, str::FromStr};

use anyhow::Context;
use clap::Args;
use colored::Colorize;
use inquire::validator::Validation;

use pesde::{
    errors::ManifestReadError, names::PackageName, scripts::ScriptName, Project, DEFAULT_INDEX_NAME,
};

use crate::cli::{config::read_config, HOME_DIR};
use fs_err::tokio as fs;

#[derive(Debug, Args)]
pub struct InitCommand {}

fn script_contents(path: &Path) -> String {
    format!(
        r#"local process = require("@lune/process")   
local home_dir = if process.os == "windows" then process.env.userprofile else process.env.HOME

require(home_dir .. {:?})"#,
        format!("/{HOME_DIR}/scripts/{}", path.display())
    )
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
            inquire::Text::new("What is the name of the project?")
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

        let description =
            inquire::Text::new("What is the description of the project? (leave empty for none)")
                .prompt()
                .unwrap();

        if !description.is_empty() {
            manifest["description"] = toml_edit::value(description);
        }

        let authors = inquire::Text::new(
            "Who are the authors of this project? (leave empty for none, comma separated)",
        )
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

        let repo = inquire::Text::new(
            "What is the repository URL of this project? (leave empty for none)",
        )
        .with_validator(|repo: &str| {
            if repo.is_empty() {
                return Ok(Validation::Valid);
            }

            Ok(match url::Url::parse(repo) {
                Ok(_) => Validation::Valid,
                Err(e) => Validation::Invalid(e.to_string().into()),
            })
        })
        .prompt()
        .unwrap();
        if !repo.is_empty() {
            manifest["repository"] = toml_edit::value(repo);
        }

        let license =
            inquire::Text::new("What is the license of this project? (leave empty for none)")
                .with_initial_value("MIT")
                .prompt()
                .unwrap();
        if !license.is_empty() {
            manifest["license"] = toml_edit::value(license);
        }

        let target_env = inquire::Select::new(
            "What environment are you targeting for your package?",
            vec!["roblox", "roblox_server", "lune", "luau"],
        )
        .prompt()
        .unwrap();

        manifest["target"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
            ["environment"] = toml_edit::value(target_env);

        if target_env == "roblox"
            || target_env == "roblox_server"
            || inquire::Confirm::new(&format!(
                "Would you like to setup a default {} script?",
                ScriptName::RobloxSyncConfigGenerator
            ))
            .prompt()
            .unwrap()
        {
            let folder = project
                .package_dir()
                .join(concat!(".", env!("CARGO_PKG_NAME")));
            fs::create_dir_all(&folder)
                .await
                .context("failed to create scripts folder")?;

            fs::write(
                folder.join(format!("{}.luau", ScriptName::RobloxSyncConfigGenerator)),
                script_contents(Path::new(&format!(
                    "lune/rojo/{}.luau",
                    ScriptName::RobloxSyncConfigGenerator
                ))),
            )
            .await
            .context("failed to write sync config generator script file")?;

            #[cfg(feature = "wally-compat")]
            fs::write(
                folder.join(format!("{}.luau", ScriptName::SourcemapGenerator)),
                script_contents(Path::new(&format!(
                    "lune/rojo/{}.luau",
                    ScriptName::SourcemapGenerator
                ))),
            )
            .await
            .context("failed to write sourcemap generator script file")?;

            let scripts =
                manifest["scripts"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()));

            scripts[&ScriptName::RobloxSyncConfigGenerator.to_string()] =
                toml_edit::value(format!(
                    concat!(".", env!("CARGO_PKG_NAME"), "/{}.luau"),
                    ScriptName::RobloxSyncConfigGenerator
                ));

            #[cfg(feature = "wally-compat")]
            {
                scripts[&ScriptName::SourcemapGenerator.to_string()] = toml_edit::value(format!(
                    concat!(".", env!("CARGO_PKG_NAME"), "/{}.luau"),
                    ScriptName::SourcemapGenerator
                ));
            }
        }

        manifest["indices"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
            [DEFAULT_INDEX_NAME] =
            toml_edit::value(read_config().await?.default_index.to_bstring().to_string());

        project.write_manifest(manifest.to_string()).await?;

        println!("{}", "initialized project".green());
        Ok(())
    }
}
