use async_trait::async_trait;
use pesde::source::pesde::registry::Entry;
use pesde::source::pesde::registry::EntryPayload;

#[async_trait]
pub trait Repository {
	async fn entry(&self, pos: u64) -> anyhow::Result<Option<Entry<EntryPayload>>>;
}
