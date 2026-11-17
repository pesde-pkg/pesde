use std::io::BufRead as _;
use std::io::PipeReader;
use std::io::Read as _;

use anyhow::Context as _;
use tokio::task::spawn_blocking;

use crate::cli::scripts::ExecuteScriptHooks;
use crate::cli::scripts::execute_script;

pub async fn authorization(script: &str) -> anyhow::Result<Option<Box<str>>> {
	let mut hooks = AuthScriptHooks {
		stdout: String::new(),
		stdout_reader: None,
		stderr_reader: None,
	};

	let status = execute_script("".into(), script, &mut hooks, vec![])
		.await
		.context("failed to execute auth script")?;
	if status != 0i32 {
		return Err(anyhow::format_err!(
			"auth script exited with non-zero status code: {status}"
		));
	}

	Ok(Some(hooks.stdout).filter(|s| !s.is_empty()).map(Into::into))
}

struct AuthScriptHooks {
	stdout: String,
	stdout_reader: Option<PipeReader>,
	stderr_reader: Option<PipeReader>,
}

impl ExecuteScriptHooks for AuthScriptHooks {
	fn stdio(
		&mut self,
	) -> (
		croshet::ShellPipeWriter,
		croshet::ShellPipeWriter,
		croshet::ShellPipeReader,
	) {
		let (stdout_reader, stdout_writer) = std::io::pipe().unwrap();
		self.stdout_reader = Some(stdout_reader);
		let (stderr_reader, stderr_writer) = std::io::pipe().unwrap();
		self.stderr_reader = Some(stderr_reader);
		let stdin_reader = std::io::pipe().unwrap().0;

		(
			croshet::ShellPipeWriter::OsPipe(stdout_writer),
			croshet::ShellPipeWriter::OsPipe(stderr_writer),
			croshet::ShellPipeReader::OsPipe(stdin_reader),
		)
	}

	async fn run(&mut self) -> anyhow::Result<()> {
		let mut stdout_reader = self.stdout_reader.take().unwrap();
		let stderr_reader = self.stderr_reader.take().unwrap();

		let (stdout, stderr) = tokio::join!(
			spawn_blocking(move || {
				let mut string = String::new();
				stdout_reader.read_to_string(&mut string).map(|_| string)
			}),
			spawn_blocking(move || {
				let stderr_reader = std::io::BufReader::new(stderr_reader);
				for line in stderr_reader.lines() {
					match line {
						Ok(line) => {
							tracing::error!("[auth script]: {line}");
						}
						Err(e) => {
							tracing::error!("ERROR IN READING STDERR OF AN AUTH SCRIPT: {e}");
						}
					}
				}
			})
		);
		self.stdout = stdout.unwrap()?;
		stderr.unwrap();

		Ok(())
	}
}
