use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use merkle_bplustree::HasherOutput;
use merkle_bplustree::TreeConfig;
use merkle_bplustree::node::TreeNode;
use merkle_bplustree::storage::ReadNodeStorage;
use merkleberg::MMRStoreReadOps;
use merkleberg::MMRStoreWriteOps;
use pesde::hash::RawHash;
use pesde::source::pesde::registry::CurrentMerkleHasher;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct StoreError(pub anyhow::Error);

#[async_trait]
pub trait MmrReadStore: Send + Sync {
	async fn get_node(&self, pos: u64) -> Result<Option<RawHash>, StoreError>;

	async fn get_nodes(&self, positions: Vec<u64>) -> Result<Vec<Option<RawHash>>, StoreError> {
		let mut nodes = Vec::with_capacity(positions.len());
		for pos in positions {
			nodes.push(self.get_node(pos).await?);
		}
		Ok(nodes)
	}
}

impl MMRStoreReadOps<RawHash> for Box<dyn MmrReadStore> {
	type Error = StoreError;

	async fn get_elem(&self, pos: u64) -> Result<Option<RawHash>, StoreError> {
		self.get_node(pos).await
	}

	async fn get_elems(
		&self,
		positions: impl Iterator<Item = u64> + Send,
	) -> Result<Vec<Option<RawHash>>, StoreError> {
		self.get_nodes(positions.collect()).await
	}
}

// explicitly not MmrReadStore to avoid hard to spot bugs
#[async_trait]
pub trait MmrWriteStore: Send + Sync + Any {
	async fn get_node(&self, pos: u64) -> Result<Option<RawHash>, StoreError>;

	async fn get_nodes(&self, positions: Vec<u64>) -> Result<Vec<Option<RawHash>>, StoreError> {
		let mut nodes = Vec::with_capacity(positions.len());
		for pos in positions {
			nodes.push(self.get_node(pos).await?);
		}
		Ok(nodes)
	}

	async fn append_nodes(&mut self, pos: u64, elems: Vec<RawHash>) -> Result<(), StoreError>;
	async fn set_size(&mut self, size: u64) -> anyhow::Result<()>;
	async fn commit(self: Box<Self>) -> anyhow::Result<()>;
}

impl MMRStoreReadOps<RawHash> for Box<dyn MmrWriteStore> {
	type Error = StoreError;

	async fn get_elem(&self, pos: u64) -> Result<Option<RawHash>, StoreError> {
		self.get_node(pos).await
	}

	async fn get_elems(
		&self,
		positions: impl Iterator<Item = u64> + Send,
	) -> Result<Vec<Option<RawHash>>, StoreError> {
		self.get_nodes(positions.collect()).await
	}
}

impl MMRStoreWriteOps<RawHash> for Box<dyn MmrWriteStore> {
	type Error = StoreError;

	async fn append(&mut self, pos: u64, elems: Vec<RawHash>) -> Result<(), StoreError> {
		self.append_nodes(pos, elems).await
	}
}

#[async_trait]
pub trait Backend:
	Send
	+ Sync
	+ crate::features::package::Repository
	+ crate::features::scope::Repository
	+ crate::features::log::Repository
	+ crate::features::search::Repository
{
	async fn begin_write(&self) -> anyhow::Result<Box<dyn MmrWriteStore>>;
}
