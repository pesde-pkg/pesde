use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    num::NonZeroUsize,
    sync::Arc,
    time::Instant,
};

use anyhow::Context;
use colored::Colorize;
use fs_err::tokio as fs;
use futures::future::try_join_all;
use pesde::{
    download_and_link::{filter_graph, DownloadAndLinkHooks, DownloadAndLinkOptions},
    lockfile::{DependencyGraph, DownloadedGraph, Lockfile},
    manifest::{target::TargetKind, DependencyType},
    Project, MANIFEST_FILE_NAME,
};
use tokio::{sync::Mutex, task::JoinSet};

use crate::cli::{
    bin_dir,
    reporters::{self, CliReporter},
    run_on_workspace_members, up_to_date_lockfile,
};

use super::files::make_executable;

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

pub struct InstallHooks {
    pub bin_folder: std::path::PathBuf,
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct InstallHooksError(#[from] anyhow::Error);

impl DownloadAndLinkHooks for InstallHooks {
    type Error = InstallHooksError;

    async fn on_bins_downloaded(
        &self,
        downloaded_graph: &pesde::lockfile::DownloadedGraph,
    ) -> Result<(), Self::Error> {
        let mut tasks = downloaded_graph
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
                let bin_folder = self.bin_folder.clone();
                let alias = alias.clone();

                async move {
                    let bin_exec_file = bin_folder
                        .join(&alias)
                        .with_extension(std::env::consts::EXE_EXTENSION);

                    let impl_folder = bin_folder.join(".impl");
                    fs::create_dir_all(&impl_folder)
                        .await
                        .context("failed to create bin link folder")?;

                    let bin_file = impl_folder.join(&alias).with_extension("luau");
                    fs::write(&bin_file, bin_link_file(&alias))
                        .await
                        .context("failed to write bin link file")?;

                    #[cfg(windows)]
                    match fs::symlink_file(
                        std::env::current_exe().context("failed to get current executable path")?,
                        &bin_exec_file,
                    )
                    .await
                    {
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                        e => e.context("failed to copy bin link file")?,
                    }

                    #[cfg(not(windows))]
                    fs::write(
                        &bin_exec_file,
                        format!(
                            r#"#!/bin/sh
exec lune run "$(dirname "$0")/.impl/{alias}.luau" -- "$@""#
                        ),
                    )
                    .await
                    .context("failed to link bin link file")?;

                    make_executable(&bin_exec_file)
                        .await
                        .context("failed to make bin link file executable")?;

                    Ok::<_, anyhow::Error>(())
                }
            })
            .collect::<JoinSet<_>>();

        while let Some(task) = tasks.join_next().await {
            task.unwrap()?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InstallOptions {
    pub locked: bool,
    pub prod: bool,
    pub write: bool,
    pub use_lockfile: bool,
    pub network_concurrency: NonZeroUsize,
}

pub async fn install(
    options: &InstallOptions,
    project: &Project,
    reqwest: reqwest::Client,
    is_root: bool,
) -> anyhow::Result<()> {
    let start = Instant::now();

    let mut refreshed_sources = HashSet::new();

    let manifest = project
        .deser_manifest()
        .await
        .context("failed to read manifest")?;

    let lockfile = if options.locked {
        match up_to_date_lockfile(project).await? {
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

    let (new_lockfile, old_graph) =
        reporters::run_with_reporter(|_, root_progress, reporter| async {
            let root_progress = root_progress;

            root_progress.set_prefix(format!("{} {}: ", manifest.name, manifest.target));
            root_progress.set_message("clean");

            if options.write {
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
                                return Err(e)
                                    .context(format!("failed to remove the {folder} folder"));
                            };

                            Ok(())
                        });
                }

                try_join_all(deleted_folders.into_values())
                    .await
                    .context("failed to remove package folders")?;
            }

            root_progress.reset();
            root_progress.set_message("resolve");

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

            let graph = project
                .dependency_graph(
                    old_graph.as_ref().filter(|_| options.use_lockfile),
                    &mut refreshed_sources,
                    false,
                )
                .await
                .context("failed to build dependency graph")?;
            let graph = Arc::new(graph);

            root_progress.reset();
            root_progress.set_length(0);
            root_progress.set_message("download");
            root_progress.set_style(reporters::root_progress_style_with_progress());

            let hooks = InstallHooks {
                bin_folder: bin_dir().await?,
            };

            let downloaded_graph = project
                .download_and_link(
                    &graph,
                    DownloadAndLinkOptions::<CliReporter, InstallHooks>::new(reqwest.clone())
                        .reporter(reporter.clone())
                        .hooks(hooks)
                        .refreshed_sources(Mutex::new(refreshed_sources))
                        .prod(options.prod)
                        .write(options.write)
                        .network_concurrency(options.network_concurrency),
                )
                .await
                .context("failed to download and link dependencies")?;

            #[cfg(feature = "patches")]
            if options.write {
                root_progress.reset();
                root_progress.set_length(0);
                root_progress.set_message("patch");

                project
                    .apply_patches(&filter_graph(&downloaded_graph, options.prod), reporter)
                    .await?;
            }

            root_progress.set_message("finish");

            let new_lockfile = Lockfile {
                name: manifest.name.clone(),
                version: manifest.version,
                target: manifest.target.kind(),
                overrides: manifest.overrides,

                graph: downloaded_graph,

                workspace: run_on_workspace_members(project, |_| async { Ok(()) }).await?,
            };

            project
                .write_lockfile(&new_lockfile)
                .await
                .context("failed to write lockfile")?;

            anyhow::Ok((new_lockfile, old_graph.unwrap_or_default()))
        })
        .await?;

    let elapsed = start.elapsed();

    if is_root {
        println!();
    }

    print_package_diff(
        &format!("{} {}:", manifest.name, manifest.target),
        old_graph,
        new_lockfile.graph,
    );

    println!("done in {:.2}s", elapsed.as_secs_f64());
    println!();

    Ok(())
}

/// Prints the difference between two graphs.
pub fn print_package_diff(prefix: &str, old_graph: DependencyGraph, new_graph: DownloadedGraph) {
    let mut old_pkg_map = BTreeMap::new();
    let mut old_direct_pkg_map = BTreeMap::new();
    let mut new_pkg_map = BTreeMap::new();
    let mut new_direct_pkg_map = BTreeMap::new();

    for (name, versions) in &old_graph {
        for (version, node) in versions {
            old_pkg_map.insert((name.clone(), version), node);
            if node.direct.is_some() {
                old_direct_pkg_map.insert((name.clone(), version), node);
            }
        }
    }

    for (name, versions) in &new_graph {
        for (version, node) in versions {
            new_pkg_map.insert((name.clone(), version), &node.node);
            if node.node.direct.is_some() {
                new_direct_pkg_map.insert((name.clone(), version), &node.node);
            }
        }
    }

    let added_pkgs = new_pkg_map
        .iter()
        .filter(|(key, _)| !old_pkg_map.contains_key(key))
        .map(|(key, &node)| (key, node))
        .collect::<Vec<_>>();
    let removed_pkgs = old_pkg_map
        .iter()
        .filter(|(key, _)| !new_pkg_map.contains_key(key))
        .map(|(key, &node)| (key, node))
        .collect::<Vec<_>>();
    let added_direct_pkgs = new_direct_pkg_map
        .iter()
        .filter(|(key, _)| !old_direct_pkg_map.contains_key(key))
        .map(|(key, &node)| (key, node))
        .collect::<Vec<_>>();
    let removed_direct_pkgs = old_direct_pkg_map
        .iter()
        .filter(|(key, _)| !new_direct_pkg_map.contains_key(key))
        .map(|(key, &node)| (key, node))
        .collect::<Vec<_>>();

    let prefix = prefix.bold();

    let no_changes = added_pkgs.is_empty()
        && removed_pkgs.is_empty()
        && added_direct_pkgs.is_empty()
        && removed_direct_pkgs.is_empty();

    if no_changes {
        println!("{prefix} already up to date");
    } else {
        let mut change_signs = [
            (!added_pkgs.is_empty()).then(|| format!("+{}", added_pkgs.len()).green().to_string()),
            (!removed_pkgs.is_empty())
                .then(|| format!("-{}", removed_pkgs.len()).red().to_string()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");

        let changes_empty = change_signs.is_empty();
        if changes_empty {
            change_signs = "(no changes)".dimmed().to_string();
        }

        println!("{prefix} {change_signs}");

        if !changes_empty {
            println!(
                "{}{}",
                "+".repeat(added_pkgs.len()).green(),
                "-".repeat(removed_pkgs.len()).red()
            );
        }

        let dependency_groups = added_direct_pkgs
            .iter()
            .map(|(key, node)| (true, key, node))
            .chain(
                removed_direct_pkgs
                    .iter()
                    .map(|(key, node)| (false, key, node)),
            )
            .filter_map(|(added, key, node)| {
                node.direct.as_ref().map(|(_, _, ty)| (added, key, ty))
            })
            .fold(
                BTreeMap::<DependencyType, BTreeSet<_>>::new(),
                |mut map, (added, key, &ty)| {
                    map.entry(ty).or_default().insert((key, added));
                    map
                },
            );

        for (ty, set) in dependency_groups {
            println!();

            let ty_name = match ty {
                DependencyType::Standard => "dependencies",
                DependencyType::Peer => "peer_dependencies",
                DependencyType::Dev => "dev_dependencies",
            };
            println!("{}", format!("{ty_name}:").yellow().bold());

            for ((name, version), added) in set {
                println!(
                    "{} {} {}",
                    if added { "+".green() } else { "-".red() },
                    name,
                    version.to_string().dimmed()
                );
            }
        }

        println!();
    }
}
