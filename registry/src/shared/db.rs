use sqlx::Database as _;
use sqlx::Sqlite;
use sqlx::SqlitePool;

#[derive(Debug, Clone)]
pub enum Database {
	Sqlite(SqlitePool),
}

impl Database {
	pub async fn new(url: &str) -> Database {
		let protocol = url.split_once(':').map_or("", |(protocol, _)| protocol);

		if Sqlite::URL_SCHEMES.contains(&protocol) {
			let pool = SqlitePool::connect(url)
				.await
				.expect("failed to connect to sqlite database");

			sqlx::migrate!("migrations/sqlite")
				.run(&pool)
				.await
				.expect("failed to migrate sqlite database");

			return Database::Sqlite(pool);
		}

		panic!("unsupported database protocol `{protocol}`")
	}
}
