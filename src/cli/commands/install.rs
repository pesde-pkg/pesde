use crate::cli::{
    bin_dir, files::make_executable, progress_bar, run_on_workspace_members, up_to_date_lockfile,
};
use anyhow::Context;
use clap::Args;
use colored::{ColoredString, Colorize};
use fs_err::tokio as fs;
use futures::future::try_join_all;
use pesde::{
    download_and_link::filter_graph, lockfile::Lockfile, manifest::target::TargetKind, Project,
    MANIFEST_FILE_NAME,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::Mutex;

#[derive(Debug, Args, Copy, Clone)]
pub struct InstallCommand {
    /// Whether to error on changes in the lockfile
    #[arg(long)]
    locked: bool,

    /// Whether to not install dev dependencies
    #[arg(long)]
    prod: bool,
}

fn bin_link_file(alias: &str) -> String {
    let mut all_combinations = BTreeSet::new();

    for a in TargetKind::VARIANTS {
        for b in TargetKind::VARIANTS {
            all_combinations.insert((a, b));
        }
    }

    let all_folders = all_combinations
        .into_iter()
        .map(|(a, b)| format!("{:?}", a.packages_folder(b)))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r#"local process = require("@lune/process")
local fs = require("@lune/fs")
local stdio = require("@lune/stdio")

local project_root = process.cwd
local path_components = string.split(string.gsub(project_root, "\\", "/"), "/")

for i = #path_components, 1, -1 do
    local path = table.concat(path_components, "/", 1, i)
    if fs.isFile(path .. "/{MANIFEST_FILE_NAME}") then
        project_root = path
        break
    end
end

for _, packages_folder in {{ {all_folders} }} do
    local path = `{{project_root}}/{{packages_folder}}/{alias}.bin.luau`
    
    if fs.isFile(path) then
        require(path)
        return
    end
end

stdio.ewrite(stdio.color("red") .. "binary `{alias}` not found. are you in the right directory?" .. stdio.color("reset") .. "\n")
    "#,
    )
}

#[cfg(feature = "patches")]
const JOBS: u8 = 5;
#[cfg(not(feature = "patches"))]
const JOBS: u8 = 4;

fn job(n: u8) -> ColoredString {
    format!("[{n}/{JOBS}]").dimmed().bold()
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
struct CallbackError(#[from] anyhow::Error);

impl InstallCommand {
    pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
        let mut refreshed_sources = HashSet::new();

        let manifest = project
            .deser_manifest()
            .await
            .context("failed to read manifest")?;

        let lockfile = if self.locked {
            match up_to_date_lockfile(&project).await? {
                None => {
                    anyhow::bail!(
                        "lockfile is out of sync, run `{} install` to update it",
                        env!("CARGO_BIN_NAME")
                    );
                }
                file => file,
            }
        } else {
            match project.deser_lockfile().await {
                Ok(lockfile) => {
                    if lockfile.overrides != manifest.overrides {
                        tracing::debug!("overrides are different");
                        None
                    } else if lockfile.target != manifest.target.kind() {
                        tracing::debug!("target kind is different");
                        None
                    } else {
                        Some(lockfile)
                    }
                }
                Err(pesde::errors::LockfileReadError::Io(e))
                    if e.kind() == std::io::ErrorKind::NotFound =>
                {
                    None
                }
                Err(e) => return Err(e.into()),
            }
        };

        println!(
            "\n{}\n",
            format!("[now installing {} {}]", manifest.name, manifest.target)
                .bold()
                .on_bright_black()
        );

        println!("{} ❌ removing current package folders", job(1));

        {
            let mut deleted_folders = HashMap::new();

            for target_kind in TargetKind::VARIANTS {
                let folder = manifest.target.kind().packages_folder(target_kind);
                let package_dir = project.package_dir();

                deleted_folders
                    .entry(folder.to_string())
                    .or_insert_with(|| async move {
                        tracing::debug!("deleting the {folder} folder");

                        if let Some(e) = fs::remove_dir_all(package_dir.join(&folder))
                            .await
                            .err()
                            .filter(|e| e.kind() != std::io::ErrorKind::NotFound)
                        {
                            return Err(e).context(format!("failed to remove the {folder} folder"));
                        };

                        Ok(())
                    });
            }

            try_join_all(deleted_folders.into_values())
                .await
                .context("failed to remove package folders")?;
        }

        let old_graph = lockfile.map(|lockfile| {
            lockfile
                .graph
                .into_iter()
                .map(|(name, versions)| {
                    (
                        name,
                        versions
                            .into_iter()
                            .map(|(version, node)| (version, node.node))
                            .collect(),
                    )
                })
                .collect()
        });

        println!("{} 📦 building dependency graph", job(2));

        let graph = project
            .dependency_graph(old_graph.as_ref(), &mut refreshed_sources, false)
            .await
            .context("failed to build dependency graph")?;
        let graph = Arc::new(graph);

        let bin_folder = bin_dir().await?;

        let downloaded_graph = {
            let (rx, downloaded_graph) = project
                .download_and_link(
                    &graph,
                    &Arc::new(Mutex::new(refreshed_sources)),
                    &reqwest,
                    self.prod,
                    true,
                    |graph| {
                        let graph = graph.clone();

                        async move {
                            try_join_all(
                                graph
                                    .values()
                                    .flat_map(|versions| versions.values())
                                    .filter(|node| node.target.bin_path().is_some())
                                    .filter_map(|node| node.node.direct.as_ref())
                                    .map(|(alias, _, _)| alias)
                                    .filter(|alias| {
                                        if *alias == env!("CARGO_BIN_NAME") {
                                            tracing::warn!(
                                            "package {alias} has the same name as the CLI, skipping bin link"
                                        );
                                            return false;
                                        }

                                        true
                                    })
                                    .map(|alias| {
                                        let bin_folder = bin_folder.clone();
                                        async move {
                                            let bin_exec_file = bin_folder.join(alias).with_extension(std::env::consts::EXE_EXTENSION);

                                            let impl_folder = bin_folder.join(".impl");
                                            fs::create_dir_all(&impl_folder).await.context("failed to create bin link folder")?;

                                            let bin_file = impl_folder.join(alias).with_extension("luau");
                                            fs::write(&bin_file, bin_link_file(alias))
                                                .await
                                                .context("failed to write bin link file")?;


                                            #[cfg(windows)]
                                            {
                                                fs::copy(
                                                    std::env::current_exe()
                                                        .context("failed to get current executable path")?,
                                                    &bin_exec_file,
                                                )
                                                    .await
                                                    .context("failed to copy bin link file")?;
                                            }

                                            #[cfg(not(windows))]
                                            {
                                                fs::write(
                                                    &bin_exec_file,
                                                    format!(r#"#!/bin/sh
exec lune run "$(dirname "$0")/.impl/{alias}.luau" -- "$@""#
                                                    ),
                                                )
                                                    .await
                                                    .context("failed to link bin link file")?;
                                            }

                                            make_executable(&bin_exec_file).await.context("failed to make bin link file executable")?;

                                            Ok::<_, CallbackError>(())
                                        }
                                    }),
                            )
                                .await
                                .map(|_| ())
                        }
                    }
                )
                .await
                .context("failed to download dependencies")?;

            progress_bar(
                graph.values().map(|versions| versions.len() as u64).sum(),
                rx,
                format!("{} 📥 ", job(3)),
                "downloading dependencies".to_string(),
                "downloaded dependencies".to_string(),
            )
            .await?;

            downloaded_graph
                .await
                .context("failed to download & link dependencies")?
        };

        #[cfg(feature = "patches")]
        {
            let rx = project
                .apply_patches(&filter_graph(&downloaded_graph, self.prod))
                .await
                .context("failed to apply patches")?;

            progress_bar(
                manifest.patches.values().map(|v| v.len() as u64).sum(),
                rx,
                format!("{} 🩹 ", job(JOBS - 1)),
                "applying patches".to_string(),
                "applied patches".to_string(),
            )
            .await?;
        }

        println!("{} 🧹 finishing up", job(JOBS));

        project
            .write_lockfile(Lockfile {
                name: manifest.name,
                version: manifest.version,
                target: manifest.target.kind(),
                overrides: manifest.overrides,

                graph: downloaded_graph,

                workspace: run_on_workspace_members(&project, |project| {
                    let reqwest = reqwest.clone();
                    async move { Box::pin(self.run(project, reqwest)).await }
                })
                .await?,
            })
            .await
            .context("failed to write lockfile")?;

        Ok(())
    }
}
