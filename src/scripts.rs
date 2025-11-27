use crate::{
	engine::runtime::{Engines, Runtime},
	manifest::Script,
	Project,
};
use futures::FutureExt as _;
use relative_path::RelativePathBuf;
use std::{
	ffi::OsStr,
	fmt::{Debug, Display, Formatter},
	path::PathBuf,
	process::Stdio,
};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tracing::instrument;

/// Script names used by pesde
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScriptName {
	/// Prints a sourcemap for a Wally package, used for finding the library export file
	#[cfg(feature = "wally-compat")]
	SourcemapGenerator,
}

impl Display for ScriptName {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			#[cfg(feature = "wally-compat")]
			ScriptName::SourcemapGenerator => write!(f, "sourcemap_generator"),
		}
	}
}

/// Extracts a script and a runtime out of a [Script]
pub fn parse_script(
	script: Script,
	engines: &Engines,
) -> Result<(Runtime, RelativePathBuf), errors::FindScriptError> {
	Ok(match script {
		Script::Path(path) => {
			let runtime = engines
				.iter()
				.filter_map(|(engine, ver)| engine.as_runtime().map(|rt| (rt, ver)))
				.collect::<Vec<_>>();
			if runtime.len() != 1 {
				return Err(errors::FindScriptError::AmbiguousRuntime);
			}

			let (runtime, version) = runtime[0];

			(Runtime::new(runtime, version.clone()), path)
		}
		Script::RuntimePath { runtime, path } => {
			let Some(version) = engines.get(&runtime.into()) else {
				return Err(errors::FindScriptError::SpecifiedRuntimeUnknown(runtime));
			};

			(Runtime::new(runtime, version.clone()), path)
		}
	})
}

/// Finds a script in the project, whether it be in the current package or it's workspace
pub async fn find_script(
	project: &Project,
	engines: &Engines,
	script_name: ScriptName,
) -> Result<Option<(Runtime, PathBuf)>, errors::FindScriptError> {
	let script_name_str = script_name.to_string();

	let (script, base) = match project
		.deser_manifest()
		.await?
		.scripts
		.remove(&script_name_str)
	{
		Some(script) => (script, project.package_dir()),
		None => match project
			.deser_workspace_manifest()
			.await?
			.and_then(|mut manifest| manifest.scripts.remove(&script_name_str))
		{
			Some(script) => (script, project.workspace_dir().unwrap()),
			None => {
				return Ok(None);
			}
		},
	};

	parse_script(script, engines).map(|(rt, path)| Some((rt, path.to_path(base))))
}

#[allow(unused_variables)]
pub(crate) trait ExecuteScriptHooks {
	fn not_found(&self, script: ScriptName) {}
}

impl ExecuteScriptHooks for () {
	#[allow(unused_variables)]
	fn not_found(&self, script: ScriptName) {}
}

#[instrument(skip(project, hooks), ret(level = "trace"), level = "debug")]
pub(crate) async fn execute_script<
	A: IntoIterator<Item = S> + Debug,
	S: AsRef<OsStr> + Debug,
	H: ExecuteScriptHooks,
>(
	script_name: ScriptName,
	project: &Project,
	engines: &Engines,
	hooks: H,
	args: A,
	return_stdout: bool,
) -> Result<Option<String>, errors::ExecuteScriptError> {
	let Some((runtime, script_path)) = find_script(project, engines, script_name).await? else {
		hooks.not_found(script_name);
		return Ok(None);
	};

	match runtime
		.prepare_command(script_path.as_os_str(), args)
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

			Ok(return_stdout.then_some(stdout_str))
		}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
			tracing::warn!("`{}` could not be found in PATH: {e}", runtime.kind());

			Ok(None)
		}
		Err(e) => Err(e.into()),
	}
}

/// Errors that can occur when using scripts
pub mod errors {
	use thiserror::Error;

	use crate::engine::runtime::RuntimeKind;

	/// Errors that can occur when finding a script
	#[derive(Debug, Error)]
	pub enum FindScriptError {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// An IO error occurred
		#[error("IO error")]
		Io(#[from] std::io::Error),

		/// Ambiguous runtime
		#[error("don't know which runtime to use. use specific form and specify the runtime")]
		AmbiguousRuntime,

		/// Runtime specified in script not in engines
		#[error("runtime `{0}` was specified in the script, but it is not present in engines")]
		SpecifiedRuntimeUnknown(RuntimeKind),
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
