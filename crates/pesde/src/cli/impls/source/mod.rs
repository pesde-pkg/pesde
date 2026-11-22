use std::collections::HashMap;
use std::io::BufRead as _;
use std::io::PipeReader;
use std::io::Read as _;
use std::num::NonZero;
use std::path::Path;
use std::path::PathBuf;
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
use tokio::sync::Semaphore;
use tokio::task::spawn_blocking;

use crate::cli::impls::sqlite::Sqlite;
use crate::cli::impls::sqlite::SqliteCachingPackageSource;
use crate::cli::registry::RegistryClient;
use crate::cli::scripts::ExecuteScriptHooks;
use crate::cli::scripts::execute_script;

mod git_index;
mod gix_git;
pub(super) mod index_legacy_pesde;
pub(super) mod index_wally;
mod registry_pesde;

pub struct CliSourceProvider {
	sources: Mutex<HashMap<SourceId, Arc<OnceCell<Arc<dyn PackageSource>>>>>,
	workspace: Workspace,
	registries: Registries,
	sqlite: Sqlite,
}

impl CliSourceProvider {
	pub fn new(workspace: Workspace, registries: Registries, sqlite: Sqlite) -> Self {
		Self {
			sources: Default::default(),
			workspace,
			registries,
			sqlite,
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
				tracing::debug!("initializing source `{source_id:?}`");

				anyhow::Ok(match source_id {
					SourceId::Pesde(url) => {
						let source = registry_pesde::RegistryPesdePackageSource::new(
							self.registries.client(url.clone()).await?,
						);

						let source = SqliteCachingPackageSource {
							source,
							sqlite: self.sqlite.clone(),
						};

						Arc::new(source) as Arc<dyn PackageSource>
					}
					SourceId::Wally(url) => {
						let source = index_wally::IndexWallyPackageSource::new(
							url.clone(),
							self.registries.network_access(url).await?,
						)
						.await?;

						let source = SqliteCachingPackageSource {
							source,
							sqlite: self.sqlite.clone(),
						};

						Arc::new(source)
					}
					SourceId::LegacyPesde(url) => {
						let source = index_legacy_pesde::IndexLegacyPesdePackageSource::new(
							url.clone(),
							self.registries.network_access(url).await?,
						)
						.await?;

						let source = SqliteCachingPackageSource {
							source,
							sqlite: self.sqlite.clone(),
						};

						Arc::new(source)
					}
					SourceId::Git(url) => {
						let source = gix_git::GixGitPackageSource::new(url.clone()).await?;

						let source = SqliteCachingPackageSource {
							source,
							sqlite: self.sqlite.clone(),
						};

						Arc::new(source)
					}
					SourceId::Workspace => {
						let source = WorkspacePackageSource::new(self.workspace.clone());

						// no SQLite caching: very hot data

						Arc::new(source)
					}
				})
			})
			.await?
			.clone();

		Ok(source)
	}
}

#[derive(Debug)]
struct NetworkAccess {
	pub reqwest: reqwest::Client,
	pub network_concurrency: Arc<Semaphore>,
	pub authorization: Option<Box<str>>,
}

#[derive(Debug, Clone)]
pub struct Registries {
	clients: Arc<Mutex<HashMap<Url, Arc<OnceCell<Arc<RegistryClient>>>>>>,
	reqwest: reqwest::Client,
	network_concurrency: Arc<Semaphore>,
	auth_scripts: Arc<HashMap<Url, Box<str>>>,
	auth_cwd: Box<Path>,
}
impl Registries {
	pub fn new(
		reqwest: reqwest::Client,
		network_concurrency: NonZero<usize>,
		auth_scripts: Arc<HashMap<Url, Box<str>>>,
		auth_cwd: Box<Path>,
	) -> Self {
		Self {
			clients: Default::default(),
			reqwest,
			network_concurrency: Arc::new(Semaphore::new(network_concurrency.get())),
			auth_scripts,
			auth_cwd,
		}
	}

	async fn network_access(&self, url: &Url) -> anyhow::Result<NetworkAccess> {
		let mut access = NetworkAccess {
			reqwest: self.reqwest.clone(),
			network_concurrency: self.network_concurrency.clone(),
			authorization: None,
		};

		let Some(script) = self.auth_scripts.get(url) else {
			return Ok(access);
		};

		access.authorization = authorization(script, self.auth_cwd.clone().into()).await?;
		Ok(access)
	}

	pub async fn client(&self, url: Url) -> anyhow::Result<Arc<RegistryClient>> {
		let cell = {
			self.clients
				.lock()
				.unwrap()
				.entry(url.clone())
				.or_default()
				.clone()
		};

		let client = cell
			.get_or_try_init(async || {
				let network_access = self.network_access(&url).await?;

				let client = RegistryClient::new(
					url,
					network_access.reqwest,
					network_access.network_concurrency,
					network_access.authorization,
				);

				anyhow::Ok(Arc::new(client))
			})
			.await?
			.clone();

		Ok(client)
	}
}

async fn authorization(script: &str, cwd: PathBuf) -> anyhow::Result<Option<Box<str>>> {
	let mut hooks = AuthScriptHooks {
		stdout: String::new(),
		stdout_reader: None,
		stderr_reader: None,
	};

	let status = execute_script(cwd, script, &mut hooks, vec![])
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
