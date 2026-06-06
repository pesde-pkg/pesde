use futures::lock::Mutex;
use sqlx::prelude::Type;
use std::sync::Arc;

use merkleberg::MMRIVER;
use merkleberg::MMRStoreReadOps;
use merkleberg::MMRStoreWriteOps;
use pesde::source::pesde::registry::*;
use sqlx::Database as _;
use sqlx::MySql;
use sqlx::MySqlPool;
use sqlx::mysql::MySqlPoolOptions;

use crate::util::AppError;

mod mysql;

#[derive(Debug, Clone)]
pub enum Database {
	MySql(MySqlPool),
}

impl Database {
	pub async fn new(url: &str) -> Database {
		let protocol = url.split_once(':').map_or("", |(protocol, _)| protocol);

		if MySql::URL_SCHEMES.contains(&protocol) {
			let pool = MySqlPoolOptions::new()
				.connect(url)
				.await
				.expect("failed to connect to mysql database");

			sqlx::migrate!()
				.run(&pool)
				.await
				.expect("failed to migrate mysql database");

			return Database::MySql(pool);
		}

		panic!("unsupported database protocol `{protocol}`")
	}

	#[must_use]
	pub fn read_mmr_sized(&self, size: u64) -> MMRIVER<CurrentMmrMerge, &Self> {
		MMRIVER::new(size, self)
	}

	pub async fn read_mmr(&self) -> anyhow::Result<MMRIVER<CurrentMmrMerge, &Self>> {
		let mmr_size = match self {
			Self::MySql(pool) => mysql::mmr_size(pool).await?,
		};

		Ok(self.read_mmr_sized(mmr_size))
	}

	pub async fn write_mmr<'t>(
		&self,
	) -> anyhow::Result<MMRIVER<CurrentMmrMerge, DatabaseTransaction<'_>>> {
		let (transaction, mmr_size) = match self {
			Self::MySql(pool) => {
				let (transaction, mmr_size) = mysql::write_mmr(pool).await?;
				(
					DatabaseTransaction::MySql(Mutex::new(transaction)),
					mmr_size,
				)
			}
		};

		Ok(MMRIVER::new(mmr_size, transaction))
	}

	pub async fn get_scope_manifest(&self, pos: u64) -> anyhow::Result<Option<ScopeManifest>> {
		match self {
			Self::MySql(pool) => mysql::get_scope_manifest(pool, pos).await,
		}
	}

	pub async fn get_scope_entry(&self, pos: u64) -> anyhow::Result<Option<ScopeEntry>> {
		match self {
			Self::MySql(pool) => mysql::get_scope_entry(pool, pos).await,
		}
	}

	pub async fn get_entry(&self, pos: u64) -> anyhow::Result<Option<Entry>> {
		match self {
			Self::MySql(pool) => mysql::get_entry(pool, pos).await,
		}
	}
}

#[derive(Debug, Type)]
#[sqlx(rename_all = "snake_case")]
pub enum EntryKind {
	Scope,
	RegisterIdentity,
	IdentityRotation,
	AdminScopeTransfer,
}

impl MMRStoreReadOps<Arc<[u8]>> for &Database {
	type Error = AppError;

	async fn get_elem(&self, pos: u64) -> Result<Option<Arc<[u8]>>, Self::Error> {
		Ok(match *self {
			Database::MySql(pool) => mysql::get_hash(pool, pos).await?,
		})
	}
}

pub trait Mmr<'a> {
	fn commit_and_update(self) -> impl Future<Output = anyhow::Result<DatabaseTransaction<'a>>>;
}

pub enum DatabaseTransaction<'a> {
	MySql(Mutex<sqlx::MySqlTransaction<'a>>),
}

impl<'a> Mmr<'a> for MMRIVER<CurrentMmrMerge, DatabaseTransaction<'a>> {
	async fn commit_and_update(mut self) -> anyhow::Result<DatabaseTransaction<'a>> {
		self.commit().await?;

		let mmr_size = self.mmr_size();
		let mut tx = self.into_store();
		match &mut tx {
			DatabaseTransaction::MySql(tx) => {
				mysql::update_mmr_size(tx.get_mut(), mmr_size).await?;
			}
		}

		Ok(tx)
	}
}

impl MMRStoreReadOps<Arc<[u8]>> for DatabaseTransaction<'_> {
	type Error = AppError;

	async fn get_elem(&self, pos: u64) -> Result<Option<Arc<[u8]>>, Self::Error> {
		Ok(match self {
			DatabaseTransaction::MySql(tx) => mysql::get_hash(&mut **tx.lock().await, pos).await?,
		})
	}
}

impl MMRStoreWriteOps<Arc<[u8]>> for DatabaseTransaction<'_> {
	type Error = AppError;

	async fn append(&mut self, pos: u64, elems: Vec<Arc<[u8]>>) -> Result<(), Self::Error> {
		match self {
			DatabaseTransaction::MySql(tx) => {
				mysql::append_hashes(&mut *tx.lock().await, pos, elems).await?;
			}
		}
		Ok(())
	}
}
