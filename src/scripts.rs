use crate::Project;
use std::{
    ffi::OsStr,
    fmt::{Display, Formatter},
    path::Path,
    process::Stdio,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

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

pub(crate) async fn execute_script<A: IntoIterator<Item = S>, S: AsRef<OsStr>>(
    script_name: ScriptName,
    script_path: &Path,
    args: A,
    project: &Project,
    return_stdout: bool,
) -> Result<Option<String>, std::io::Error> {
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

            let script = script_name.to_string();
            let script_2 = script.to_string();

            tokio::spawn(async move {
                while let Some(line) = stderr.next_line().await.transpose() {
                    match line {
                        Ok(line) => {
                            log::error!("[{script}]: {line}");
                        }
                        Err(e) => {
                            log::error!("ERROR IN READING STDERR OF {script}: {e}");
                            break;
                        }
                    }
                }
            });

            let mut stdout_str = String::new();

            while let Some(line) = stdout.next_line().await.transpose() {
                match line {
                    Ok(line) => {
                        if return_stdout {
                            stdout_str.push_str(&line);
                            stdout_str.push('\n');
                        } else {
                            log::info!("[{script_2}]: {line}");
                        }
                    }
                    Err(e) => {
                        log::error!("ERROR IN READING STDOUT OF {script_2}: {e}");
                        break;
                    }
                }
            }

            if return_stdout {
                Ok(Some(stdout_str))
            } else {
                Ok(None)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("Lune could not be found in PATH: {e}");

            Ok(None)
        }
        Err(e) => Err(e),
    }
}
