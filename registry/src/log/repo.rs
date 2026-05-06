use async_trait::async_trait;
use sqlx::SqlitePool;

#[async_trait]
pub trait LogRepo: Send + Sync + 'static {
	/// Gets the head of the log
	async fn head(&self) -> anyhow::Result<()>;

	/// Gets the consistency proof between two log sizes
	async fn consistency(&self) -> anyhow::Result<()>;

	/// Gets the inclusion proof for a given leaf index and tree size
	async fn inclusion(&self) -> anyhow::Result<()>;
}

pub struct SqliteLogRepo {
	pool: SqlitePool,
}

impl SqliteLogRepo {
	pub fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}
}

#[async_trait]
impl LogRepo for SqliteLogRepo {
	async fn head(&self) -> anyhow::Result<()> {}

	async fn consistency(&self) -> anyhow::Result<()> {}

	async fn inclusion(&self) -> anyhow::Result<()> {}
}
