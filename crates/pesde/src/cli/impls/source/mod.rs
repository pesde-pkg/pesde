use std::collections::HashMap;
use std::num::NonZero;
use std::sync::Arc;
use std::sync::Mutex;

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

use crate::cli::auth::authorization;
use crate::cli::impls::sqlite::Sqlite;
use crate::cli::impls::sqlite::SqliteCachingPackageSource;

mod git_index;
mod gix_git;
pub(super) mod index_legacy_pesde;
pub(super) mod index_wally;
mod registry_pesde;

pub struct CliSourceProvider {
	sources: Mutex<HashMap<SourceId, Arc<OnceCell<Arc<dyn PackageSource>>>>>,
	workspace: Workspace,
	sqlite: Sqlite,
	reqwest: reqwest::Client,
	network_concurrency: Arc<Semaphore>,
	auth_scripts: Arc<HashMap<Url, Box<str>>>,
}

impl CliSourceProvider {
	pub fn new(
		workspace: Workspace,
		sqlite: Sqlite,
		reqwest: reqwest::Client,
		network_concurrency: NonZero<usize>,
		auth_scripts: Arc<HashMap<Url, Box<str>>>,
	) -> Self {
		Self {
			sources: Default::default(),
			workspace,
			sqlite,
			reqwest,
			network_concurrency: Arc::new(Semaphore::new(network_concurrency.get())),
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
				let network_access = async |url: &Url| -> anyhow::Result<NetworkAccess> {
					let mut access = NetworkAccess {
						reqwest: self.reqwest.clone(),
						network_concurrency: self.network_concurrency.clone(),
						authorization: None,
					};

					let Some(script) = self.auth_scripts.get(url) else {
						return Ok(access);
					};

					access.authorization = authorization(script).await?;
					Ok(access)
				};

				tracing::debug!("initializing source `{source_id:?}`");

				anyhow::Ok(match source_id {
					SourceId::Pesde(url) => {
						let source = registry_pesde::RegistryPesdePackageSource::new(
							url.clone(),
							network_access(url).await?,
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
							network_access(url).await?,
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
							network_access(url).await?,
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

struct NetworkAccess {
	pub reqwest: reqwest::Client,
	pub network_concurrency: Arc<Semaphore>,
	pub authorization: Option<Box<str>>,
}
