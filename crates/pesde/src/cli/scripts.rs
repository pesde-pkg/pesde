use croshet::ShellPipeReader;
use croshet::ShellPipeWriter;
use std::future;
use std::path::PathBuf;
use tracing::instrument;

pub trait ExecuteScriptHooks {
	fn stdio(&mut self) -> (ShellPipeWriter, ShellPipeWriter, ShellPipeReader) {
		(
			ShellPipeWriter::Stdout,
			ShellPipeWriter::Stderr,
			ShellPipeReader::stdin(),
		)
	}

	fn run(&mut self) -> impl Future<Output = anyhow::Result<()>> {
		future::ready(Ok(()))
	}
}

impl ExecuteScriptHooks for () {}

#[instrument(skip(hooks), level = "debug")]
pub async fn execute_script<H: ExecuteScriptHooks>(
	cwd: PathBuf,
	script: &str,
	hooks: &mut H,
	args: Vec<std::ffi::OsString>,
) -> anyhow::Result<i32> {
	let parsed_script = croshet::parser::parse(script)?;

	let (stdout, stderr, stdin) = hooks.stdio();
	let mut options = croshet::ExecuteOptionsBuilder::new()
		.cwd(cwd)
		.stdout(stdout)
		.stderr(stderr)
		.stdin(stdin)
		.args(args)
		.build()
		.unwrap();
	options.env_vars = std::env::vars_os().collect();

	let (code, stdio_result) = tokio::join!(croshet::execute(parsed_script, options), hooks.run());
	stdio_result?;

	Ok(code)
}
