pub struct Repos {}

async fn sqlite(url: &str) -> Repos {
	let pool = match sqlx::SqlitePool::connect(url).await {
		Ok(pool) => pool,
		Err(e) => panic!("failed to create sqlite pool: {e}"),
	};

	Repos {}
}

impl Repos {
	pub async fn new(url: &str) -> Self {
		if url.starts_with("sqlite://") {
			sqlite(url).await
		} else {
			panic!("unknown database type")
		}
	}
}
