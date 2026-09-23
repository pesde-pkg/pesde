use std::num::NonZero;

use async_trait::async_trait;
use merkleberg::MMRStoreReadOps;
use merkleberg::MMRStoreWriteOps;
use pesde::signature::PublicKey;
use pesde::source::pesde::registry::*;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct StoreError(pub anyhow::Error);

#[async_trait]
pub trait MmrReadStore: Send + Sync {
	async fn get_node(&self, pos: u64) -> Result<Option<CurrentHash>, StoreError>;

	async fn get_nodes(&self, positions: Vec<u64>) -> Result<Vec<Option<CurrentHash>>, StoreError> {
		let mut nodes = Vec::new();
		nodes.reserve_exact(positions.len());
		for pos in positions {
			nodes.push(self.get_node(pos).await?);
		}
		Ok(nodes)
	}
}

impl MMRStoreReadOps<CurrentHash> for &dyn MmrReadStore {
	type Error = StoreError;

	async fn get_elem(&self, pos: u64) -> Result<Option<CurrentHash>, StoreError> {
		self.get_node(pos).await
	}

	async fn get_elems(
		&self,
		positions: impl Iterator<Item = u64> + Send,
	) -> Result<Vec<Option<CurrentHash>>, StoreError> {
		self.get_nodes(positions.collect()).await
	}
}

#[async_trait]
pub trait MmrWriteStore: MmrReadStore {
	async fn append_nodes(&mut self, pos: u64, elems: Vec<CurrentHash>) -> Result<(), StoreError>;
	async fn set_size(&mut self, size: u64) -> anyhow::Result<()>;
}

impl MMRStoreReadOps<CurrentHash> for &dyn MmrWriteStore {
	type Error = StoreError;

	async fn get_elem(&self, pos: u64) -> Result<Option<CurrentHash>, Self::Error> {
		(*self as &dyn MmrReadStore).get_elem(pos).await
	}

	async fn get_elems(
		&self,
		positions: impl Iterator<Item = u64> + Send,
	) -> Result<Vec<Option<CurrentHash>>, Self::Error> {
		(*self as &dyn MmrReadStore).get_elems(positions).await
	}
}

impl MMRStoreWriteOps<CurrentHash> for dyn MmrWriteStore {
	type Error = StoreError;

	async fn append(&mut self, pos: u64, elems: Vec<CurrentHash>) -> Result<(), StoreError> {
		self.append_nodes(pos, elems).await
	}
}

pub enum PermissionWidth<'a> {
	OnlySelf,
	Package(&'a LocalNameId),
	Owner,
}

pub enum ExistingScopeLockResult {
	Ok {
		tx: Box<dyn ScopeWriteTransaction>,
		scope_size: NonZero<u64>,
	},
	Unauthorized,
	DoesntExist,
}

pub enum CreatingScopeLockResult {
	Ok {
		tx: Box<dyn GlobalWriteTransaction>,
		global_size: u64,
	},
	AlreadyExists,
}

#[async_trait]
pub trait Backend:
	Send
	+ Sync
	+ crate::features::log::GlobalReadRepository
	+ crate::features::scope::ScopeReadRepository
{
	fn global_mmr_read_store(&self) -> Box<dyn MmrReadStore>;

	fn scope_mmr_read_store(&self, scope_id: &ScopeId) -> Box<dyn MmrReadStore>;

	async fn begin_write_existing(
		&self,
		scope_id: &ScopeId,
		user: &PublicKey,
		permission_width: PermissionWidth<'_>,
	) -> anyhow::Result<ExistingScopeLockResult>;

	async fn begin_write_creating(
		&self,
		scope_id: &ScopeId,
	) -> anyhow::Result<CreatingScopeLockResult>;
}

#[async_trait]
pub trait ScopeWriteTransaction:
	MmrWriteStore + crate::features::scope::ScopeWriteRepository
{
	async fn commit(self: Box<Self>) -> anyhow::Result<()>;
	async fn rollback(self: Box<Self>) -> anyhow::Result<()>;
}

#[async_trait]
pub trait GlobalWriteTransaction:
	MmrWriteStore + crate::features::log::GlobalWriteRepository
{
	fn into_scope_transaction(self: Box<Self>) -> Box<dyn ScopeWriteTransaction>;

	async fn rollback(self: Box<Self>) -> anyhow::Result<()>;
}
