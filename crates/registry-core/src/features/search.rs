use async_trait::async_trait;
use futures::stream::BoxStream;
use pesde::names::PackageName;
use pesde::source::pesde::registry::SearchResultItem;

#[derive(Debug)]
pub struct SearchPackage {
	pub id: u64,
	pub pos: u64,
	pub item: SearchResultItem,
}

#[async_trait]
pub trait Repository {
	async fn all_packages_for_index(&self) -> BoxStream<'_, anyhow::Result<SearchPackage>>;

	async fn searchable_version(&self, name: &PackageName) -> anyhow::Result<SearchPackage>;

	async fn search_result_by_pos(&self, pos: u64) -> anyhow::Result<SearchResultItem>;
}
