use sqlx::Database as _;
use sqlx::MySql;
use sqlx::MySqlPool;

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
}
