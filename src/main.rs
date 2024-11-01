#[cfg(feature = "version-management")]
use crate::cli::version::{
    check_for_updates, current_version, get_or_download_version, max_installed_version,
};
use crate::cli::{auth::get_tokens, home_dir, repos::update_repo_dependencies, HOME_DIR};
use anyhow::Context;
use clap::Parser;
use colored::Colorize;
use indicatif::MultiProgress;
use indicatif_log_bridge::LogWrapper;
use pesde::{AuthConfig, Project, MANIFEST_FILE_NAME};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    thread::spawn,
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

fn get_linkable_dir(path: &Path) -> PathBuf {
    let mut curr_path = PathBuf::new();
    let file_to_try = NamedTempFile::new_in(path).expect("failed to create temporary file");
    let temp_file_name = file_to_try.path().file_name().unwrap();

    for component in path.components() {
        curr_path.push(component);

        let try_path = curr_path.join(temp_file_name);

        if fs_err::hard_link(file_to_try.path(), &try_path).is_ok() {
            if let Err(err) = fs_err::remove_file(&try_path) {
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

fn run() -> anyhow::Result<()> {
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

        fn get_workspace_members(path: &Path) -> anyhow::Result<HashSet<PathBuf>> {
            let manifest = fs_err::read_to_string(path.join(MANIFEST_FILE_NAME))
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
                    if get_workspace_members(&path)?.contains(project_root) {
                        workspace_dir = Some(path);
                    }
                }

                (None, None) => {
                    if get_workspace_members(&path)?.contains(&cwd) {
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
    fs_err::create_dir_all(&data_dir).expect("failed to create data directory");

    let cas_dir = get_linkable_dir(&project_root_dir).join(HOME_DIR);

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
        AuthConfig::new().with_tokens(get_tokens()?.0),
    );

    let reqwest = {
        let mut headers = reqwest::header::HeaderMap::new();

        headers.insert(
            reqwest::header::ACCEPT,
            "application/json"
                .parse()
                .context("failed to create accept header")?,
        );

        reqwest::blocking::Client::builder()
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
            .ok()
            .and_then(|manifest| manifest.pesde_version);

        // store the current version in case it needs to be used later
        get_or_download_version(&reqwest, &current_version())?;

        let exe_path = if let Some(version) = target_version {
            Some(get_or_download_version(&reqwest, &version)?)
        } else {
            None
        };
        let exe_path = if let Some(exe_path) = exe_path {
            exe_path
        } else {
            get_or_download_version(&reqwest, &max_installed_version()?)?
        };

        if let Some(exe_path) = exe_path {
            let status = std::process::Command::new(exe_path)
                .args(std::env::args_os().skip(1))
                .status()
                .expect("failed to run new version");

            std::process::exit(status.code().unwrap());
        }

        display_err(check_for_updates(&reqwest), " while checking for updates");
    }

    let project_2 = project.clone();
    let update_task = spawn(move || {
        display_err(
            update_repo_dependencies(&project_2),
            " while updating repository dependencies",
        );
    });

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let _ = err.print();
            update_task.join().expect("failed to join update task");
            std::process::exit(err.exit_code());
        }
    };

    cli.subcommand.run(project, multi, reqwest, update_task)
}

fn display_err(result: anyhow::Result<()>, prefix: &str) {
    if let Err(err) = result {
        eprintln!("{}: {err}\n", format!("error{prefix}").red().bold());

        let cause = err.chain().skip(1).collect::<Vec<_>>();

        if !cause.is_empty() {
            eprintln!("{}:", "caused by".red().bold());
            for err in cause {
                eprintln!("  - {err}");
            }
        }

        let backtrace = err.backtrace();
        match backtrace.status() {
            std::backtrace::BacktraceStatus::Disabled => {
                eprintln!(
                    "\n{}: set RUST_BACKTRACE=1 for a backtrace",
                    "help".yellow().bold()
                );
            }
            std::backtrace::BacktraceStatus::Captured => {
                eprintln!("\n{}:\n{backtrace}", "backtrace".yellow().bold());
            }
            _ => {
                eprintln!("\n{}: not captured", "backtrace".yellow().bold());
            }
        }
    }
}

fn main() {
    let result = run();
    let is_err = result.is_err();
    display_err(result, "");
    if is_err {
        std::process::exit(1);
    }
}
