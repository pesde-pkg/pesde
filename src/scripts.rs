use croshet::ShellPipeReader;
use croshet::ShellPipeWriter;
use std::convert::Infallible;
use std::error::Error;
use std::future;
use tracing::instrument;

use crate::Subproject;

/// Hooks for [execute_script]
#[allow(unused_variables)]
pub trait ExecuteScriptHooks {
	/// The error the methods return
	type Error: Error;

	/// Returns the stdio options in the format of (stdout, stderr, stdin)
	fn stdio(&mut self) -> (ShellPipeWriter, ShellPipeWriter, ShellPipeReader) {
		(
			ShellPipeWriter::Stdout,
			ShellPipeWriter::Stderr,
			ShellPipeReader::stdin(),
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
#[instrument(skip(hooks), level = "debug")]
pub async fn execute_script<H: ExecuteScriptHooks>(
	subproject: &Subproject,
	script: &str,
	hooks: &mut H,
	args: Vec<std::ffi::OsString>,
) -> Result<i32, errors::ExecuteScriptError<H>> {
	let parsed_script = croshet::parser::parse(script)?;

	let (stdout, stderr, stdin) = hooks.stdio();

	let (code, stdio_result) = tokio::join!(
		croshet::execute(
			parsed_script,
			croshet::ExecuteOptionsBuilder::new()
				.cwd(subproject.dir())
				.stdout(stdout)
				.stderr(stderr)
				.stdin(stdin)
				.args(args)
				.build()
				.unwrap(),
		),
		hooks.run(),
	);
	stdio_result.map_err(errors::ExecuteScriptError::Hooks)?;

	Ok(code)
}

/// Errors that can occur when using scripts
pub mod errors {
	use thiserror::Error;

	use crate::scripts::ExecuteScriptHooks;

	/// Errors which can occur while executing a script
	#[derive(Debug, Error)]
	pub enum ExecuteScriptError<Hooks: ExecuteScriptHooks> {
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
