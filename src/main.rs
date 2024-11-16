#[cfg(feature = "version-management")]
use crate::cli::version::{check_for_updates, get_or_download_version};
use crate::cli::{auth::get_tokens, display_err, home_dir, HOME_DIR};
use anyhow::Context;
use clap::Parser;
use fs_err::tokio as fs;
use indicatif::MultiProgress;
use indicatif_log_bridge::LogWrapper;
use pesde::{AuthConfig, Project, MANIFEST_FILE_NAME};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

mod cli;
pub mod util;

#[derive(Parser, Debug)]
#[clap(
    version,
    about = "A package manager for the Luau programming language, supporting multiple runtimes including Roblox and Lune"
)]
#[command(disable_version_flag = true)]
struct Cli {
    /// Print version
    #[arg(short = 'v', short_alias = 'V', long, action = clap::builder::ArgAction::Version)]
    version: (),

    #[command(subcommand)]
    subcommand: cli::commands::Subcommand,
}

async fn get_linkable_dir(path: &Path) -> PathBuf {
    let mut curr_path = PathBuf::new();
    let file_to_try = NamedTempFile::new_in(path).expect("failed to create temporary file");

    let temp_path = tempfile::Builder::new()
        .make(|_| Ok(()))
        .expect("failed to create temporary file")
        .into_temp_path();
    let temp_file_name = temp_path.file_name().expect("failed to get file name");

    // C: and \ are different components on Windows
    #[cfg(windows)]
    let components = path.components().map(|c| {
        let mut path = c.as_os_str().to_os_string();
        if let std::path::Component::Prefix(_) = c {
            path.push(std::path::MAIN_SEPARATOR_STR);
        }

        path
    });
    #[cfg(not(windows))]
    let components = path.components().map(|c| c.as_os_str().to_os_string());

    for component in components {
        curr_path.push(component);

        let try_path = curr_path.join(temp_file_name);

        if fs::hard_link(file_to_try.path(), &try_path).await.is_ok() {
            if let Err(err) = fs::remove_file(&try_path).await {
                log::warn!(
                    "failed to remove temporary file at {}: {err}",
                    try_path.display()
                );
            }

            return curr_path;
        }
    }

    panic!(
        "couldn't find a linkable directory for any point in {}",
        curr_path.display()
    );
}

async fn run() -> anyhow::Result<()> {
    let cwd = std::env::current_dir().expect("failed to get current working directory");

    #[cfg(windows)]
    'scripts: {
        let exe = std::env::current_exe().expect("failed to get current executable path");
        if exe.parent().is_some_and(|parent| {
            parent.file_name().is_some_and(|parent| parent != "bin")
                || parent
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .is_some_and(|parent| parent != HOME_DIR)
        }) {
            break 'scripts;
        }

        let exe_name = exe.file_name().unwrap().to_string_lossy();
        let exe_name = exe_name
            .strip_suffix(std::env::consts::EXE_SUFFIX)
            .unwrap_or(&exe_name);

        if exe_name == env!("CARGO_BIN_NAME") {
            break 'scripts;
        }

        // the bin script will search for the project root itself, so we do that to ensure
        // consistency across platforms, since the script is executed using a shebang
        // on unix systems
        let status = std::process::Command::new("lune")
            .arg("run")
            .arg(exe.with_extension(""))
            .arg("--")
            .args(std::env::args_os().skip(1))
            .current_dir(cwd)
            .status()
            .expect("failed to run lune");

        std::process::exit(status.code().unwrap());
    }

    let (project_root_dir, project_workspace_dir) = 'finder: {
        let mut current_path = Some(cwd.clone());
        let mut project_root = None::<PathBuf>;
        let mut workspace_dir = None::<PathBuf>;

        async fn get_workspace_members(path: &Path) -> anyhow::Result<HashSet<PathBuf>> {
            let manifest = fs::read_to_string(path.join(MANIFEST_FILE_NAME))
                .await
                .context("failed to read manifest")?;
            let manifest: pesde::manifest::Manifest =
                toml::from_str(&manifest).context("failed to parse manifest")?;

            if manifest.workspace_members.is_empty() {
                return Ok(HashSet::new());
            }

            manifest
                .workspace_members
                .iter()
                .map(|member| path.join(member))
                .map(|p| glob::glob(&p.to_string_lossy()))
                .collect::<Result<Vec<_>, _>>()
                .context("invalid glob patterns")?
                .into_iter()
                .flat_map(|paths| paths.into_iter())
                .collect::<Result<HashSet<_>, _>>()
                .context("failed to expand glob patterns")
        }

        while let Some(path) = current_path {
            current_path = path.parent().map(|p| p.to_path_buf());

            if !path.join(MANIFEST_FILE_NAME).exists() {
                continue;
            }

            match (project_root.as_ref(), workspace_dir.as_ref()) {
                (Some(project_root), Some(workspace_dir)) => {
                    break 'finder (project_root.clone(), Some(workspace_dir.clone()));
                }

                (Some(project_root), None) => {
                    if get_workspace_members(&path).await?.contains(project_root) {
                        workspace_dir = Some(path);
                    }
                }

                (None, None) => {
                    if get_workspace_members(&path).await?.contains(&cwd) {
                        // initializing a new member of a workspace
                        break 'finder (cwd, Some(path));
                    } else {
                        project_root = Some(path);
                    }
                }

                (None, Some(_)) => unreachable!(),
            }
        }

        // we mustn't expect the project root to be found, as that would
        // disable the ability to run pesde in a non-project directory (for example to init it)
        (project_root.unwrap_or_else(|| cwd.clone()), workspace_dir)
    };

    let multi = {
        let logger = pretty_env_logger::formatted_builder()
            .parse_env(pretty_env_logger::env_logger::Env::default().default_filter_or("info"))
            .build();
        let multi = MultiProgress::new();

        LogWrapper::new(multi.clone(), logger).try_init().unwrap();

        multi
    };

    let home_dir = home_dir()?;
    let data_dir = home_dir.join("data");
    fs::create_dir_all(&data_dir)
        .await
        .expect("failed to create data directory");

    let cas_dir = get_linkable_dir(&project_root_dir).await.join(HOME_DIR);

    let cas_dir = if cas_dir == home_dir {
        &data_dir
    } else {
        &cas_dir
    }
    .join("cas");

    log::debug!("using cas dir in {}", cas_dir.display());

    let project = Project::new(
        project_root_dir,
        project_workspace_dir,
        data_dir,
        cas_dir,
        AuthConfig::new().with_tokens(get_tokens().await?.0),
    );

    let reqwest = {
        let mut headers = reqwest::header::HeaderMap::new();

        headers.insert(
            reqwest::header::ACCEPT,
            "application/json"
                .parse()
                .context("failed to create accept header")?,
        );

        reqwest::Client::builder()
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .default_headers(headers)
            .build()?
    };

    #[cfg(feature = "version-management")]
    {
        let target_version = project
            .deser_manifest()
            .await
            .ok()
            .and_then(|manifest| manifest.pesde_version);

        let exe_path = if let Some(version) = target_version {
            get_or_download_version(&reqwest, &version, false).await?
        } else {
            None
        };

        if let Some(exe_path) = exe_path {
            let status = std::process::Command::new(exe_path)
                .args(std::env::args_os().skip(1))
                .status()
                .expect("failed to run new version");

            std::process::exit(status.code().unwrap());
        }

        display_err(
            check_for_updates(&reqwest).await,
            " while checking for updates",
        );
    }

    let cli = Cli::parse();

    cli.subcommand.run(project, multi, reqwest).await
}

#[tokio::main]
async fn main() {
    let result = run().await;
    let is_err = result.is_err();
    display_err(result, "");
    if is_err {
        std::process::exit(1);
    }
}
