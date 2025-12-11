use crate::Project;
use std::{
	fmt::{Debug, Display, Formatter},
	io::{BufRead as _, BufReader, Read as _},
};
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

/// Finds a script in the project, whether it be in the current package or it's workspace
pub async fn find_script(
	project: &Project,
	script_name: ScriptName,
) -> Result<Option<String>, errors::FindScriptError> {
	let script_name_str = script_name.to_string();

	Ok(
		match project
			.deser_manifest()
			.await?
			.scripts
			.remove(&script_name_str)
		{
			Some(script) => Some(script),
			None => project
				.deser_workspace_manifest()
				.await?
				.and_then(|mut manifest| manifest.scripts.remove(&script_name_str)),
		},
	)
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
pub(crate) async fn execute_script<H: ExecuteScriptHooks>(
	script_name: ScriptName,
	project: &Project,
	hooks: H,
	args: Vec<std::ffi::OsString>,
	return_stdout: bool,
) -> Result<Option<String>, errors::ExecuteScriptError> {
	let Some(script) = find_script(project, script_name).await? else {
		hooks.not_found(script_name);
		return Ok(None);
	};

	let parsed_script = croshet::parser::parse(&script)?;

	let (stdout_reader, stdout_writer) = std::io::pipe()?;
	let (stderr_reader, stderr_writer) = std::io::pipe()?;

	let read_future = return_stdout.then(|| {
		let mut stdout_reader_str = stdout_reader.try_clone().unwrap();
		tokio::task::spawn_blocking(move || {
			let mut str = String::new();
			stdout_reader_str.read_to_string(&mut str).map(|_| str)
		})
	});

	let (code, stdout_err, stderr_err) = tokio::join!(
		croshet::execute(
			parsed_script,
			croshet::ExecuteOptionsBuilder::new()
				.cwd(project.package_dir().to_path_buf())
				.args(args)
				.stdout(croshet::ShellPipeWriter::OsPipe(stdout_writer))
				.stderr(croshet::ShellPipeWriter::OsPipe(stderr_writer))
				.build()
				.unwrap()
		),
		async {
			if return_stdout {
				Ok(())
			} else {
				tokio::task::spawn_blocking(move || {
					let stdout = BufReader::new(stdout_reader).lines();
					for line in stdout {
						match line {
							Ok(line) => {
								tracing::info!("[{script_name}]: {line}");
							}
							Err(e) => {
								tracing::error!("ERROR IN READING STDOUT OF {script_name}: {e}");
							}
						}
					}
				})
				.await
			}
		},
		{
			let script_name = script_name;
			tokio::task::spawn_blocking(move || {
				let stderr = BufReader::new(stderr_reader).lines();
				for line in stderr {
					match line {
						Ok(line) => {
							tracing::error!("[{script_name}]: {line}");
						}
						Err(e) => {
							tracing::error!("ERROR IN READING STDERR OF {script_name}: {e}");
						}
					}
				}
			})
		}
	);
	stdout_err.unwrap();
	stderr_err.unwrap();
	if code != 0i32 {
		return Err(errors::ExecuteScriptError::Io(std::io::Error::other(
			format!("script {script_name} exited with non-zero code {code}"),
		)));
	}

	Ok(if let Some(read_future) = read_future {
		let stdout = read_future.await.unwrap()?;
		Some(stdout)
	} else {
		None
	})
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

		/// Script parsing failed
		#[error("script parsing failed")]
		Parse(#[from] croshet::Error),
	}
}
