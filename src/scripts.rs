use crate::Project;
use futures::FutureExt;
use std::{
    ffi::OsStr,
    fmt::{Debug, Display, Formatter},
    path::PathBuf,
    process::Stdio,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tracing::instrument;

/// Script names used by pesde
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScriptName {
    /// Generates a config for syncing tools for Roblox. For example, for Rojo it should create a `default.project.json` file
    RobloxSyncConfigGenerator,
    /// Prints a sourcemap for a Wally package, used for finding the library export file
    #[cfg(feature = "wally-compat")]
    SourcemapGenerator,
}

impl Display for ScriptName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptName::RobloxSyncConfigGenerator => write!(f, "roblox_sync_config_generator"),
            #[cfg(feature = "wally-compat")]
            ScriptName::SourcemapGenerator => write!(f, "sourcemap_generator"),
        }
    }
}

/// Finds a script in the project, whether it be in the current package or it's workspace
pub async fn find_script(
    project: &Project,
    script_name: ScriptName,
) -> Result<Option<PathBuf>, errors::FindScriptError> {
    let script_name_str = script_name.to_string();

    let script_path = match project
        .deser_manifest()
        .await?
        .scripts
        .remove(&script_name_str)
    {
        Some(script) => script.to_path(project.package_dir()),
        None => match project
            .deser_workspace_manifest()
            .await?
            .and_then(|mut manifest| manifest.scripts.remove(&script_name_str))
        {
            Some(script) => script.to_path(project.workspace_dir().unwrap()),
            None => {
                return Ok(None);
            }
        },
    };

    Ok(Some(script_path))
}

#[allow(unused_variables)]
pub(crate) trait ExecuteScriptHooks {
    fn not_found(&self, script: ScriptName) {}
}

#[instrument(skip(project, hooks), level = "debug")]
pub(crate) async fn execute_script<
    A: IntoIterator<Item = S> + Debug,
    S: AsRef<OsStr> + Debug,
    H: ExecuteScriptHooks,
>(
    script_name: ScriptName,
    project: &Project,
    hooks: H,
    args: A,
    return_stdout: bool,
) -> Result<Option<String>, errors::ExecuteScriptError> {
    let Some(script_path) = find_script(project, script_name).await? else {
        hooks.not_found(script_name);
        return Ok(None);
    };

    match Command::new("lune")
        .arg("run")
        .arg(script_path.as_os_str())
        .arg("--")
        .args(args)
        .current_dir(project.package_dir())
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(mut child) => {
            let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
            let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();

            let mut stdout_str = String::new();

            loop {
                tokio::select! {
                    Some(line) = stdout.next_line().map(Result::transpose) => match line {
                        Ok(line) => {
                            if return_stdout {
                                stdout_str.push_str(&line);
                                stdout_str.push('\n');
                            } else {
                                tracing::info!("[{script_name}]: {line}");
                            }
                        }
                        Err(e) => {
                            tracing::error!("ERROR IN READING STDOUT OF {script_name}: {e}");
                        }
                    },
                    Some(line) = stderr.next_line().map(Result::transpose) => match line {
                        Ok(line) => {
                            tracing::error!("[{script_name}]: {line}");
                        }
                        Err(e) => {
                            tracing::error!("ERROR IN READING STDERR OF {script_name}: {e}");
                        }
                    },
                    else => break,
                }
            }

            if return_stdout {
                Ok(Some(stdout_str))
            } else {
                Ok(None)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!("Lune could not be found in PATH: {e}");

            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}

/// Errors that can occur when using scripts
pub mod errors {
    use thiserror::Error;

    /// Errors that can occur when finding a script
    #[derive(Debug, Error)]
    pub enum FindScriptError {
        /// Reading the manifest failed
        #[error("error reading manifest")]
        ManifestRead(#[from] crate::errors::ManifestReadError),

        /// An IO error occurred
        #[error("IO error")]
        Io(#[from] std::io::Error),
    }

    /// Errors which can occur while executing a script
    #[derive(Debug, Error)]
    pub enum ExecuteScriptError {
        /// Finding the script failed
        #[error("finding the script failed")]
        FindScript(#[from] FindScriptError),

        /// An IO error occurred
        #[error("IO error")]
        Io(#[from] std::io::Error),
    }
}
