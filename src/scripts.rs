use crate::Project;
use std::{
	convert::Infallible,
	env::{join_paths, split_paths},
	error::Error,
	future,
};
use tracing::instrument;

/// Prints a sourcemap for a Wally package, used for finding the library export file
#[cfg(feature = "wally-compat")]
pub const SOURCEMAP_GENERATOR: &str = "sourcemap_generator";

/// Hooks for [execute_script]
#[allow(unused_variables)]
pub trait ExecuteScriptHooks {
	/// The error the methods return
	type Error: Error;

	/// Returns the stdio options in the format of (stdout, stderr, stdin)
	fn stdio(
		&mut self,
	) -> (
		croshet::ShellPipeWriter,
		croshet::ShellPipeWriter,
		croshet::ShellPipeReader,
	) {
		(
			croshet::ShellPipeWriter::Stdout,
			croshet::ShellPipeWriter::Stderr,
			croshet::ShellPipeReader::stdin(),
		)
	}

	/// Called when the script is being executed
	fn run(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
		future::ready(Ok(()))
	}
}

impl ExecuteScriptHooks for () {
	type Error = Infallible;
}

/// Executes a script
#[instrument(skip(project, hooks), level = "debug")]
pub async fn execute_script<H: ExecuteScriptHooks>(
	script_name: &str,
	project: &Project,
	hooks: &mut H,
	args: Vec<std::ffi::OsString>,
) -> Result<bool, errors::ExecuteScriptError<H>> {
	let Some(script) = project.deser_manifest().await?.scripts.remove(script_name) else {
		return Ok(false);
	};

	let parsed_script = croshet::parser::parse(&script)?;

	let mut paths = vec![project.bin_dir().to_path_buf()];
	if std::env::var("PESDE_IMPURE_SCRIPTS").is_ok_and(|s| !s.is_empty())
		&& let Some(path) = std::env::var_os("PATH")
	{
		paths.extend(split_paths(&path));
	}
	let path = join_paths(paths)?;

	let (stdout, stderr, stdin) = hooks.stdio();

	let (code, stdio_result) = tokio::join!(
		croshet::execute(
			parsed_script,
			croshet::ExecuteOptionsBuilder::new()
				.cwd(project.package_dir().to_path_buf())
				.stdout(stdout)
				.stderr(stderr)
				.stdin(stdin)
				.args(args)
				.env_var("PATH".into(), path)
				.build()
				.unwrap(),
		),
		hooks.run(),
	);
	stdio_result.map_err(errors::ExecuteScriptError::Hooks)?;
	if code != 0i32 {
		return Err(errors::ExecuteScriptError::Io(std::io::Error::other(
			format!("script {script_name} exited with non-zero code {code}"),
		)));
	}

	Ok(true)
}

/// Errors that can occur when using scripts
pub mod errors {
	use thiserror::Error;

	use crate::scripts::ExecuteScriptHooks;

	/// Errors which can occur while executing a script
	#[derive(Debug, Error)]
	pub enum ExecuteScriptError<Hooks: ExecuteScriptHooks> {
		/// An IO error occurred
		#[error("IO error")]
		Io(#[from] std::io::Error),

		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// Constructing a PATH failed
		#[error("PATH creation error")]
		Path(#[from] std::env::JoinPathsError),

		/// Script parsing failed
		#[error("script parsing failed")]
		Parse(#[from] croshet::Error),

		/// The hooks have errored
		#[error("error executing hook")]
		Hooks(Hooks::Error),
	}
}
