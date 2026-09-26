use async_trait::async_trait;
use pesde::source::pesde::registry::*;

#[async_trait]
pub trait GlobalReadRepository {
	async fn global_log_size(&self) -> anyhow::Result<u64>;

	async fn global_log_entry(&self, pos: u64) -> anyhow::Result<Option<GlobalEntry>>;
}

#[async_trait]
pub trait GlobalWriteRepository: GlobalReadRepository {
	async fn insert_entry(&mut self, entry: &GlobalEntry) -> anyhow::Result<()>;
}
