use merkleberg::MMRIVER;
use merkleberg::MMRStoreReadOps;
use merkleberg::MMRStoreWriteOps;
use pesde::hash::Hash;
use pesde::source::pesde::registry::*;
use sqlx::Database as _;
use sqlx::Executor as _;
use sqlx::MySql;
use sqlx::MySqlPool;
use sqlx::mysql::MySqlPoolOptions;

use crate::util::AnyhowError;

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
				.after_connect(|conn, _meta| {
					Box::pin(async move {
						conn.execute("SET autocommit = 0").await?;
						conn.execute("SET SESSION TRANSACTION ISOLATION LEVEL SERIALIZABLE")
							.await?;
						Ok(())
					})
				})
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
	pub fn read_mmr_sized(&self, size: u64) -> MMRIVER<Sha256Merge, &Self> {
		MMRIVER::new(size, self)
	}

	pub async fn read_mmr(&self) -> anyhow::Result<MMRIVER<Sha256Merge, &Self>> {
		let mmr_size = match self {
			Self::MySql(pool) => mysql::mmr_size(pool).await?,
		};

		Ok(self.read_mmr_sized(mmr_size))
	}

	pub async fn write_mmr(&self) -> anyhow::Result<MMRIVER<Sha256Merge, DatabaseTransaction<'_>>> {
		let (transaction, mmr_size) = match self {
			Self::MySql(pool) => {
				let (transaction, mmr_size) = mysql::write_mmr(pool).await?;
				(DatabaseTransaction::MySql(transaction), mmr_size)
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

impl MMRStoreReadOps<Hash> for &Database {
	type Error = AnyhowError;

	async fn get_elem(&self, pos: u64) -> Result<Option<Hash>, Self::Error> {
		Ok(match *self {
			Database::MySql(pool) => mysql::get_hash(pool, pos).await?,
		})
	}
}

pub enum DatabaseTransaction<'a> {
	MySql(sqlx::MySqlTransaction<'a>),
}

impl MMRStoreWriteOps<Hash> for DatabaseTransaction<'_> {
	type Error = AnyhowError;

	async fn append(&mut self, pos: u64, elems: Vec<Hash>) -> Result<(), Self::Error> {
		match self {
			DatabaseTransaction::MySql(tx) => mysql::append_hashes(tx, pos, elems).await?,
		}
		Ok(())
	}
}
