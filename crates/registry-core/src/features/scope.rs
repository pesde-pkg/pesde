use async_trait::async_trait;
use pesde::source::pesde::registry::*;

#[async_trait]
pub trait Repository {}

#[async_trait]
pub trait ScopeWriteRepository {}

#[async_trait]
pub trait GlobalWriteRepository {
	async fn insert_global_entry(&mut self, global_entry: GlobalEntry) -> anyhow::Result<()>;
}
