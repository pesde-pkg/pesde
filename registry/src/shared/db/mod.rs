use merkleberg::MMRIVER;
use merkleberg::MMRStoreReadOps;
use pesde::hash::Hash;
use pesde::source::pesde::registry::Sha256Merge;
use sqlx::Database as _;
use sqlx::MySql;
use sqlx::MySqlPool;

use crate::util::AnyhowError;

pub mod mysql;

#[derive(Debug, Clone)]
pub enum Database {
	MySql(MySqlPool),
}

impl Database {
	pub async fn new(url: &str) -> Database {
		let protocol = url.split_once(':').map_or("", |(protocol, _)| protocol);

		if MySql::URL_SCHEMES.contains(&protocol) {
			let pool = MySqlPool::connect(url)
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

	pub async fn read_mmr(&self) -> anyhow::Result<MMRIVER<Sha256Merge, &Self>> {
		let mmr_size = match self {
			Self::MySql(pool) => mysql::mmr_size(pool).await?,
		};

		Ok(MMRIVER::new(mmr_size, self))
	}
}

impl MMRStoreReadOps<Hash> for &Database {
	type Error = AnyhowError;

	async fn get_elem(&self, pos: u64) -> Result<Option<Hash>, Self::Error> {
		Ok(None)
	}
}
