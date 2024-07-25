use crate::cli::{home_dir, reqwest_client, IsUpToDate};
use anyhow::Context;
use clap::Args;
use indicatif::MultiProgress;
use pesde::{lockfile::Lockfile, manifest::target::TargetKind, Project};
use std::{
    collections::{BTreeSet, HashSet},
    sync::Arc,
    time::Duration,
};

#[derive(Debug, Args)]
pub struct InstallCommand {
    /// The amount of threads to use for downloading
    #[arg(short, long, default_value_t = 6, value_parser = clap::value_parser!(u64).range(1..=128))]
    threads: u64,
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

    #[cfg(windows)]
    let prefix = String::new();
    #[cfg(not(windows))]
    let prefix = "#!/usr/bin/env -S lune run\n";

    format!(
        r#"{prefix}local process = require("@lune/process")
local fs = require("@lune/fs")
    
for _, packages_folder in {{ {all_folders} }} do
    local path = `{{process.cwd}}/{{packages_folder}}/{alias}.bin.luau`
    
    if fs.isFile(path) then
        require(path)
        break
    end
end
    "#,
    )
}

impl InstallCommand {
    pub fn run(self, project: Project, multi: MultiProgress) -> anyhow::Result<()> {
        let mut refreshed_sources = HashSet::new();

        let manifest = project
            .deser_manifest()
            .context("failed to read manifest")?;

        let lockfile = if project
            .is_up_to_date(false)
            .context("failed to check if project is up to date")?
        {
            match project.deser_lockfile() {
                Ok(lockfile) => Some(lockfile),
                Err(pesde::errors::LockfileReadError::Io(e))
                    if e.kind() == std::io::ErrorKind::NotFound =>
                {
                    None
                }
                Err(e) => return Err(e.into()),
            }
        } else {
            None
        };

        {
            let mut deleted_folders = HashSet::new();

            for target_kind in TargetKind::VARIANTS {
                let folder = manifest.target.kind().packages_folder(target_kind);

                if deleted_folders.insert(folder.to_string()) {
                    log::debug!("deleting the {folder} folder");

                    if let Some(e) = std::fs::remove_dir_all(project.path().join(&folder))
                        .err()
                        .filter(|e| e.kind() != std::io::ErrorKind::NotFound)
                    {
                        return Err(e).context(format!("failed to remove the {folder} folder"));
                    };
                }
            }
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

        let graph = project
            .dependency_graph(old_graph.as_ref(), &mut refreshed_sources)
            .context("failed to build dependency graph")?;

        let bar = multi.add(
            indicatif::ProgressBar::new(graph.values().map(|versions| versions.len() as u64).sum())
                .with_style(
                    indicatif::ProgressStyle::default_bar().template(
                        "{msg} {bar:40.208/166} {pos}/{len} {percent}% {elapsed_precise}",
                    )?,
                )
                .with_message("downloading dependencies"),
        );
        bar.enable_steady_tick(Duration::from_millis(100));

        let (rx, downloaded_graph) = project
            .download_graph(
                &graph,
                &mut refreshed_sources,
                &reqwest_client(project.data_dir())?,
                self.threads as usize,
            )
            .context("failed to download dependencies")?;

        while let Ok(result) = rx.recv() {
            bar.inc(1);

            match result {
                Ok(()) => {}
                Err(e) => return Err(e.into()),
            }
        }

        bar.finish_with_message("finished downloading dependencies");

        let downloaded_graph = Arc::into_inner(downloaded_graph)
            .unwrap()
            .into_inner()
            .unwrap();

        project
            .link_dependencies(&downloaded_graph)
            .context("failed to link dependencies")?;

        project
            .apply_patches(&downloaded_graph)
            .context("failed to apply patches")?;

        let bin_folder = home_dir()?.join("bin");

        for versions in downloaded_graph.values() {
            for node in versions.values() {
                if node.target.bin_path().is_none() {
                    continue;
                }

                let Some((alias, _)) = &node.node.direct else {
                    continue;
                };

                let bin_file = bin_folder.join(format!("{alias}.luau"));
                std::fs::write(&bin_file, bin_link_file(alias))
                    .context("failed to write bin link file")?;

                // TODO: test if this actually works
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;

                    let mut perms = std::fs::metadata(&bin_file)
                        .context("failed to get bin link file metadata")?
                        .permissions();
                    perms.set_mode(perms.mode() | 0o111);
                    std::fs::set_permissions(&bin_file, perms)
                        .context("failed to set bin link file permissions")?;
                }

                #[cfg(windows)]
                {
                    let bin_file = bin_file.with_extension(std::env::consts::EXE_EXTENSION);
                    std::fs::copy(
                        std::env::current_exe().context("failed to get current executable path")?,
                        &bin_file,
                    )
                    .context("failed to copy bin link file")?;
                }
            }
        }

        project
            .write_lockfile(Lockfile {
                name: manifest.name,
                version: manifest.version,
                target: manifest.target.kind(),
                overrides: manifest.overrides,

                graph: downloaded_graph,
            })
            .context("failed to write lockfile")?;

        Ok(())
    }
}
