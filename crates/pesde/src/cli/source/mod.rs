use std::collections::HashMap;
use std::io::BufRead as _;
use std::io::PipeReader;
use std::io::Read as _;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Context as _;
use async_trait::async_trait;
use pesde::Url;
use pesde::Workspace;
use pesde::source::PackageSource;
use pesde::source::SourceError;
use pesde::source::SourceId;
use pesde::source::SourceProvider;
use pesde::source::WorkspacePackageSource;
use tokio::sync::OnceCell;
use tokio::task::spawn_blocking;

use crate::cli::scripts::ExecuteScriptHooks;
use crate::cli::scripts::execute_script;

mod git_index;
mod gix_git;
mod index_legacy_pesde;
mod index_wally;
mod local_package;
mod registry_pesde;

pub struct CliSourceProvider {
	sources: Mutex<HashMap<SourceId, Arc<OnceCell<Arc<dyn PackageSource>>>>>,
	workspace: Workspace,
	reqwest: reqwest::Client,
	auth_scripts: HashMap<Url, Box<str>>,
}

impl CliSourceProvider {
	pub fn new(
		workspace: Workspace,
		reqwest: reqwest::Client,
		auth_scripts: HashMap<Url, Box<str>>,
	) -> Self {
		Self {
			sources: Default::default(),
			workspace,
			reqwest,
			auth_scripts,
		}
	}
}

#[async_trait]
impl SourceProvider for CliSourceProvider {
	async fn get(&self, source_id: &SourceId) -> Result<Arc<dyn PackageSource>, SourceError> {
		let cell = {
			self.sources
				.lock()
				.unwrap()
				.entry(source_id.clone())
				.or_default()
				.clone()
		};

		let source = cell
			.get_or_try_init(async || {
				let get_authorization = async |url: &Url| -> anyhow::Result<Option<Box<str>>> {
					let Some(script) = self.auth_scripts.get(url) else {
						return Ok(None);
					};

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

					Ok(Some(hooks.stdout.into_boxed_str()).filter(|s| !s.is_empty()))
				};

				tracing::debug!("initializing source `{source_id:?}`");

				anyhow::Ok(match source_id {
					SourceId::Pesde(url) => Arc::new(registry_pesde::RegistryPesdePackageSource {
						url: url.clone(),
						reqwest: self.reqwest.clone(),
						authorization: get_authorization(url).await?,
					}) as Arc<dyn PackageSource>,
					SourceId::Wally(url) => Arc::new(
						index_wally::IndexWallyPackageSource::new(
							url.clone(),
							self.reqwest.clone(),
							get_authorization(url).await?,
						)
						.await?,
					),
					SourceId::LegacyPesde(url) => Arc::new(
						index_legacy_pesde::IndexLegacyPesdePackageSource::new(
							url.clone(),
							self.reqwest.clone(),
							get_authorization(url).await?,
						)
						.await?,
					),
					SourceId::Git(url) => {
						Arc::new(gix_git::GixGitPackageSource::new(url.clone()).await?)
					}
					SourceId::Workspace => {
						Arc::new(WorkspacePackageSource::new(self.workspace.clone()))
					}
				})
			})
			.await?
			.clone();

		Ok(source)
	}
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
