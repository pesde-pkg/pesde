use async_trait::async_trait;
use merkleberg::{MMRStoreReadOps, MMRStoreWriteOps};
use pesde::source::pesde::registry::*;

use crate::db::StoreError;

#[async_trait]
pub trait GlobalReadRepository: Send + Sync {
	async fn global_log_size(&self) -> anyhow::Result<u64>;

	async fn global_log_entry(&self, pos: u64) -> anyhow::Result<Option<GlobalEntry>>;

	async fn global_log_node(&self, pos: u64) -> anyhow::Result<Option<CurrentHash>>;

	async fn global_log_nodes(
		&self,
		positions: Vec<u64>,
	) -> anyhow::Result<Vec<Option<CurrentHash>>>;
}

#[async_trait]
pub trait GlobalWriteRepository: GlobalReadRepository {
	async fn append_global_log_nodes(
		&mut self,
		pos: u64,
		elems: Vec<CurrentHash>,
	) -> anyhow::Result<()>;

	async fn set_global_log_size(&mut self, size: u64) -> anyhow::Result<()>;

	async fn insert_global_entry(&mut self, entry: &GlobalEntry) -> anyhow::Result<()>;
}

macro_rules! forward_global_read {
	($($ty:ty),+ $(,)?) => {$(
		#[async_trait]
		impl<T: GlobalReadRepository + ?Sized> GlobalReadRepository for $ty {
			async fn global_log_size(&self) -> anyhow::Result<u64> {
				(**self).global_log_size().await
			}

			async fn global_log_entry(&self, pos: u64) -> anyhow::Result<Option<GlobalEntry>> {
				(**self).global_log_entry(pos).await
			}

			async fn global_log_node(&self, pos: u64) -> anyhow::Result<Option<CurrentHash>> {
				(**self).global_log_node(pos).await
			}

			async fn global_log_nodes(
				&self,
				positions: Vec<u64>,
			) -> anyhow::Result<Vec<Option<CurrentHash>>> {
				(**self).global_log_nodes(positions).await
			}
		}
	)+};
}

macro_rules! forward_global_write {
	($($ty:ty),+ $(,)?) => {$(
		#[async_trait]
		impl<T: GlobalWriteRepository + ?Sized> GlobalWriteRepository for $ty {
			async fn append_global_log_nodes(
				&mut self,
				pos: u64,
				elems: Vec<CurrentHash>,
			) -> anyhow::Result<()> {
				(**self).append_global_log_nodes(pos, elems).await
			}

			async fn set_global_log_size(&mut self, size: u64) -> anyhow::Result<()> {
				(**self).set_global_log_size(size).await
			}

			async fn insert_global_entry(&mut self, entry: &GlobalEntry) -> anyhow::Result<()> {
				(**self).insert_global_entry(entry).await
			}
		}
	)+};
}

forward_global_read!(&T, &mut T, Box<T>);
forward_global_write!(&mut T, Box<T>);

pub struct Global<R>(pub R);

impl<R: GlobalReadRepository> MMRStoreReadOps<CurrentHash> for Global<R> {
	type Error = StoreError;

	async fn get_elem(&self, pos: u64) -> Result<Option<CurrentHash>, StoreError> {
		self.0.global_log_node(pos).await.map_err(StoreError)
	}

	async fn get_elems(
		&self,
		positions: impl Iterator<Item = u64> + Send,
	) -> Result<Vec<Option<CurrentHash>>, StoreError> {
		self.0
			.global_log_nodes(positions.collect())
			.await
			.map_err(StoreError)
	}
}

impl<R: GlobalWriteRepository> MMRStoreWriteOps<CurrentHash> for Global<R> {
	type Error = StoreError;

	async fn append(&mut self, pos: u64, elems: Vec<CurrentHash>) -> Result<(), StoreError> {
		self.0
			.append_global_log_nodes(pos, elems)
			.await
			.map_err(StoreError)
	}
}
