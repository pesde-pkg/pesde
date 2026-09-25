use async_trait::async_trait;
use merkle_bplustree::{
	HasherOutput, TreeConfig,
	hasher::Hasher,
	node::TreeNode,
	storage::{ReadNodeStorage, WriteNodeStorage},
};
use pesde::source::pesde::registry::CurrentHash;

use crate::db::StoreError;

#[async_trait]
pub trait TreeReadRepository<C: TreeConfig>: Send + Sync {
	async fn get_node(&self, hash: &CurrentHash) -> anyhow::Result<TreeNode<C>>;
}

impl<C: TreeConfig> ReadNodeStorage<C> for &dyn TreeReadRepository<C>
where
	C::Hasher: Hasher<C::Key, C::Value, Output = CurrentHash>,
{
	type Error = StoreError;

	async fn get_node(
		&self,
		hash: &merkle_bplustree::HasherOutput<C>,
	) -> Result<TreeNode<C>, Self::Error> {
		(*self as &dyn TreeReadRepository<C>)
			.get_node(hash)
			.await
			.map_err(StoreError)
	}
}

#[async_trait]
pub trait TreeWriteRepository<C: TreeConfig>: TreeReadRepository<C> {
	async fn store_nodes(
		&mut self,
		nodes: Vec<(HasherOutput<C>, TreeNode<C>)>,
	) -> anyhow::Result<()>;
}

impl<C: TreeConfig> ReadNodeStorage<C> for &mut dyn TreeWriteRepository<C>
where
	C::Hasher: Hasher<C::Key, C::Value, Output = CurrentHash>,
{
	type Error = StoreError;

	async fn get_node(
		&self,
		hash: &merkle_bplustree::HasherOutput<C>,
	) -> Result<TreeNode<C>, Self::Error> {
		ReadNodeStorage::get_node(&(*self as &dyn TreeReadRepository<C>), hash).await
	}
}

impl<C: TreeConfig> WriteNodeStorage<C> for &mut dyn TreeWriteRepository<C>
where
	C::Hasher: Hasher<C::Key, C::Value, Output = CurrentHash>,
{
	async fn store_nodes(
		&mut self,
		nodes: impl Iterator<Item = (merkle_bplustree::HasherOutput<C>, TreeNode<C>)> + Send,
	) -> Result<(), Self::Error> {
		(*self as &mut dyn TreeWriteRepository<C>)
			.store_nodes(nodes.collect())
			.await
			.map_err(StoreError)
	}
}
