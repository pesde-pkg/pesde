use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;
use futures::Stream;
use futures::StreamExt as _;
use pesde::reporters::DownloadProgressReporter;
use pesde::source::DependencySpecifier;
use pesde::source::InspectResult;
use pesde::source::PackageId;
use pesde::source::PackageSource;
use pesde::source::ResolveResult;
use pesde::source::SourceError;
use pesde::vfs::PackageStorer;
use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;
use tokio::sync::OnceCell;
use tracing::instrument;

use crate::cli::impls::vfs::CasPackageLayoutManifest;

#[derive(Debug, Clone)]
pub struct Sqlite {
	pool: SqlitePool,
	migrate: Arc<OnceCell<()>>,
}

impl Sqlite {
	pub fn new(in_dir: &Path) -> Self {
		let options = sqlx::sqlite::SqliteConnectOptions::new()
			.create_if_missing(true)
			.journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
			.synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
			.filename(in_dir.join("index.sqlite"));

		Self {
			pool: SqlitePoolOptions::new()
				.max_connections(10)
				.connect_lazy_with(options),
			migrate: Default::default(),
		}
	}

	async fn migrate(&self) -> anyhow::Result<()> {
		self.migrate
			.get_or_try_init(|| async {
				sqlx::migrate!()
					.run(&self.pool)
					.await
					.context("failed to run SQLite migrations")?;

				anyhow::Ok(())
			})
			.await?;
		Ok(())
	}

	async fn cache_inspect(
		&self,
		package_id: &PackageId,
		inspect_result: &InspectResult,
	) -> anyhow::Result<()> {
		// TODO: we needn't reproducible semantics here, check if cbor-core is worth replacing
		let inspection_bytes = cbor_core::Value::serialized(inspect_result)
			.context("failed to serialize InspectResult for caching")?
			.encode();

		sqlx::query!(
			r#"
INSERT INTO package_cache (id, inspection, layout)
VALUES (?1, ?2, X'')
ON CONFLICT(id) DO UPDATE SET layout = excluded.layout
            "#,
			package_id.to_string(),
			inspection_bytes,
		)
		.execute(&self.pool)
		.await?;

		Ok(())
	}

	async fn get_cached_inspect(
		&self,
		package_id: &PackageId,
	) -> anyhow::Result<Option<InspectResult>> {
		let row = sqlx::query!(
			"SELECT inspection FROM package_cache WHERE id = ?1",
			package_id.to_string()
		)
		.fetch_optional(&self.pool)
		.await
		.context("failed to fetch cached InspectResult")?;

		let Some(row) = row.filter(|r| !r.inspection.is_empty()) else {
			return Ok(None);
		};

		let inspect_result = cbor_core::Value::decode_owned(row.inspection)
			.context("failed to decode InspectResult from cache")?
			.deserialized()
			.context("failed to deserialize InspectResult from cache")?;
		Ok(Some(inspect_result))
	}

	pub async fn get_cached_layout(
		&self,
		package_id: &PackageId,
	) -> anyhow::Result<Option<CasPackageLayoutManifest>> {
		let row = sqlx::query!(
			"SELECT layout FROM package_cache WHERE id = ?1",
			package_id.to_string()
		)
		.fetch_optional(&self.pool)
		.await
		.context("failed to fetch cached layout")?;

		let Some(layout) = row.filter(|r| !r.layout.is_empty()) else {
			return Ok(None);
		};

		let layout = cbor_core::Value::decode_owned(layout.layout)
			.context("failed to decode layout from cache")?
			.deserialized()
			.context("failed to deserialize layout from cache")?;

		Ok(Some(layout))
	}

	pub async fn cache_layout(
		&self,
		package_id: &PackageId,
		layout: &CasPackageLayoutManifest,
	) -> anyhow::Result<()> {
		let layout_bytes = cbor_core::Value::serialized(layout)
			.context("failed to serialize layout for caching")?
			.encode();

		sqlx::query!(
			r#"
INSERT INTO package_cache (id, inspection, layout)
VALUES (?1, X'', ?2)
ON CONFLICT(id) DO UPDATE SET layout = excluded.layout
            "#,
			package_id.to_string(),
			layout_bytes,
		)
		.execute(&self.pool)
		.await?;

		Ok(())
	}

	pub async fn list_cached_packages(
		&self,
	) -> anyhow::Result<
		SqliteCacheManageStream<
			impl Stream<Item = anyhow::Result<(PackageId, Option<CasPackageLayoutManifest>)>>,
		>,
	> {
		self.migrate().await?;

		// a separate connection so we can safely delete as we read. possible due to WAL mode
		let delete_conn = self
			.pool
			.acquire()
			.await
			.context("failed to acquire connection for deleting cached packages")?;

		let rows = sqlx::query!("SELECT id, layout FROM package_cache")
			.fetch(&self.pool)
			.map(|result| {
				let row = result.context("failed to fetch cached package")?;

				let package_id = row
					.id
					.parse()
					.map_err(|e| anyhow::format_err!("failed to parse cached package id: {e}"))?;

				let layout = if row.layout.is_empty() {
					None
				} else {
					let layout = cbor_core::Value::decode_owned(row.layout)
						.context("failed to decode cached layout")?
						.deserialized()
						.context("failed to deserialize cached layout")?;

					Some(layout)
				};

				Ok((package_id, layout))
			});

		Ok(SqliteCacheManageStream {
			stream: rows,
			delete_conn,
			current_item: None,
		})
	}
}

pub struct SqliteCacheManageStream<S> {
	stream: S,
	delete_conn: sqlx::pool::PoolConnection<sqlx::Sqlite>,
	current_item: Option<PackageId>,
}

impl<S: Stream<Item = anyhow::Result<(PackageId, Option<CasPackageLayoutManifest>)>> + Unpin>
	SqliteCacheManageStream<S>
{
	pub async fn next(
		&mut self,
	) -> anyhow::Result<Option<(&'_ PackageId, Option<CasPackageLayoutManifest>)>> {
		let item = self.stream.next().await.transpose()?;
		let Some((package_id, layout)) = item else {
			self.current_item = None;
			return Ok(None);
		};

		let current_item = self.current_item.insert(package_id);
		Ok(Some((current_item, layout)))
	}

	pub async fn delete_current_item(&mut self) -> anyhow::Result<()> {
		let Some(package_id) = self.current_item.take() else {
			anyhow::bail!("no current item to delete");
		};

		sqlx::query!(
			"DELETE FROM package_cache WHERE id = ?1",
			package_id.to_string()
		)
		.execute(&mut *self.delete_conn)
		.await
		.context("failed to delete cached package")?;

		Ok(())
	}
}

pub struct SqliteCachingPackageSource<S: PackageSource> {
	pub sqlite: Sqlite,
	pub source: S,
}

#[async_trait]
impl<S: PackageSource> PackageSource for SqliteCachingPackageSource<S> {
	// resolution itself isn't cached, at least at this layer - we don't know if the source has changed
	// we do, however, cache the InspectResult as it should be deterministic for a given package id
	#[instrument(skip(self), level = "debug")]
	async fn resolve(&self, specifier: &DependencySpecifier) -> Result<ResolveResult, SourceError> {
		let result = self.source.resolve(specifier).await?;
		let stream = match result {
			ResolveResult::Found(stream) => stream,
			ResolveResult::Elsewhere(sources) => return Ok(ResolveResult::Elsewhere(sources)),
		};

		let sqlite = self.sqlite.clone();
		sqlite.migrate().await?;
		let stream = stream.then(move |result| {
			let sqlite = sqlite.clone();
			async move {
				let (package_id, inspect_result) = result?;

				if let Some(inspect_result) = inspect_result.as_ref() {
					sqlite.cache_inspect(&package_id, inspect_result).await?;
				}

				Ok((package_id, inspect_result))
			}
		});

		Ok(ResolveResult::Found(stream.boxed()))
	}

	// delegate caching the download to the vfs, it has commit and it's overall its job to do this
	#[instrument(skip(self, reporter, to), level = "debug")]
	async fn download(
		&self,
		package_id: &PackageId,
		reporter: Arc<dyn DownloadProgressReporter + 'static>,
		to: &mut dyn PackageStorer,
	) -> Result<(), SourceError> {
		self.source.download(package_id, reporter, to).await
	}

	#[instrument(skip(self), level = "debug")]
	async fn inspect(&self, package_id: &PackageId) -> Result<InspectResult, SourceError> {
		self.sqlite.migrate().await?;
		if let Some(inspect_result) = self.sqlite.get_cached_inspect(package_id).await? {
			tracing::debug!("cache hit");
			return Ok(inspect_result);
		}

		tracing::debug!("cache miss");

		let inspect_result = self.source.inspect(package_id).await?;
		self.sqlite
			.cache_inspect(package_id, &inspect_result)
			.await
			.context("failed to cache InspectResult")?;
		Ok(inspect_result)
	}
}
