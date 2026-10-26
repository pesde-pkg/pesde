use std::num::NonZero;
use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;
use merkle_bplustree::HasherOutput;
use merkle_bplustree::TreeConfig;
use merkle_bplustree::hasher::Hasher;
use merkle_bplustree::node::TreeNode;
use merkle_bplustree::storage::ReadNodeStorage;
use merkle_bplustree::storage::WriteNodeStorage;
use merkleberg::MMRStoreReadOps;
use merkleberg::MMRStoreWriteOps;
use pesde::hash::Hash;
use pesde::source::pesde::registry::*;

use crate::db::StoreError;

#[async_trait]
pub trait ScopeReadRepository:
	Send
	+ Sync
	+ ScopeTreeReadRepository<ScopeMembersTree>
	+ ScopeTreeReadRepository<PackageVersionsTree>
	+ ScopeTreeReadRepository<PackageDeprecationsTree>
{
	async fn scope_log_size(&self, scope: &ScopeId) -> anyhow::Result<Option<NonZero<u64>>>;

	async fn scope_log_entry(&self, scope: ScopeId, pos: u64)
	-> anyhow::Result<Option<ScopeEntry>>;

	async fn scope_state(&self, scope: &ScopeId) -> anyhow::Result<Option<ScopeStateResponse>>;

	async fn scope_log_node(
		&self,
		scope: &ScopeId,
		pos: u64,
	) -> anyhow::Result<Option<CurrentHash>>;

	async fn scope_log_nodes(
		&self,
		scope: &ScopeId,
		positions: Vec<u64>,
	) -> anyhow::Result<Vec<Option<CurrentHash>>>;
}

#[async_trait]
pub trait ScopeTreeReadRepository<T: TreeConfig>: Send + Sync {
	async fn scope_tree_node(&self, hash: &CurrentHash) -> anyhow::Result<Option<TreeNode<T>>>;
}

#[async_trait]
pub trait ScopeWriteRepository:
	ScopeReadRepository
	+ ScopeTreeWriteRepository<ScopeMembersTree>
	+ ScopeTreeWriteRepository<PackageVersionsTree>
	+ ScopeTreeWriteRepository<PackageDeprecationsTree>
{
	async fn append_scope_log_nodes(
		&mut self,
		scope: &ScopeId,
		pos: u64,
		elems: Vec<CurrentHash>,
	) -> anyhow::Result<()>;

	async fn set_scope_log_size(&mut self, scope: &ScopeId, size: u64) -> anyhow::Result<()>;

	async fn insert_package_version(
		&mut self,
		scope: &ScopeId,
		pos: u64,
		package_version: &PackageVersion,
		archive_hash: &Hash,
	) -> anyhow::Result<()>;

	async fn insert_scope_entry(&mut self, entry: &ScopeEntry) -> anyhow::Result<()>;
}

#[async_trait]
pub trait ScopeTreeWriteRepository<T: TreeConfig>: ScopeTreeReadRepository<T> {
	async fn store_scope_nodes(
		&mut self,
		scope: &ScopeId,
		nodes: Vec<(HasherOutput<T>, TreeNode<T>)>,
	) -> anyhow::Result<()>;
}

macro_rules! forward_scope_read {
	($($ty:ty),+ $(,)?) => {$(
		#[async_trait]
		impl<T: ScopeReadRepository + ?Sized> ScopeReadRepository for $ty {
			async fn scope_log_size(
				&self,
				scope: &ScopeId,
			) -> anyhow::Result<Option<NonZero<u64>>> {
				(**self).scope_log_size(scope).await
			}

			async fn scope_log_entry(
				&self,
				scope: ScopeId,
				pos: u64,
			) -> anyhow::Result<Option<ScopeEntry>> {
				(**self).scope_log_entry(scope, pos).await
			}

			async fn scope_state(
				&self,
				scope: &ScopeId,
			) -> anyhow::Result<Option<ScopeStateResponse>> {
				(**self).scope_state(scope).await
			}

			async fn scope_log_node(
				&self,
				scope: &ScopeId,
				pos: u64,
			) -> anyhow::Result<Option<CurrentHash>> {
				(**self).scope_log_node(scope, pos).await
			}

			async fn scope_log_nodes(
				&self,
				scope: &ScopeId,
				positions: Vec<u64>,
			) -> anyhow::Result<Vec<Option<CurrentHash>>> {
				(**self).scope_log_nodes(scope, positions).await
			}
		}

		#[async_trait]
		impl<C: TreeConfig, T: ScopeTreeReadRepository<C> + ?Sized> ScopeTreeReadRepository<C>
			for $ty
		{
			async fn scope_tree_node(
				&self,
				hash: &CurrentHash,
			) -> anyhow::Result<Option<TreeNode<C>>> {
				(**self).scope_tree_node(hash).await
			}
		}
	)+};
}

macro_rules! forward_scope_write {
	($($ty:ty),+ $(,)?) => {$(
		#[async_trait]
		impl<T: ScopeWriteRepository + ?Sized> ScopeWriteRepository for $ty {
			async fn append_scope_log_nodes(
				&mut self,
				scope: &ScopeId,
				pos: u64,
				elems: Vec<CurrentHash>,
			) -> anyhow::Result<()> {
				(**self).append_scope_log_nodes(scope, pos, elems).await
			}

			async fn set_scope_log_size(
				&mut self,
				scope: &ScopeId,
				size: u64,
			) -> anyhow::Result<()> {
				(**self).set_scope_log_size(scope, size).await
			}

			async fn insert_package_version(
				&mut self,
				scope: &ScopeId,
				pos: u64,
				package_version: &PackageVersion,
				archive_hash: &Hash,
			) -> anyhow::Result<()> {
				(**self)
					.insert_package_version(scope, pos, package_version, archive_hash)
					.await
			}

			async fn insert_scope_entry(&mut self, entry: &ScopeEntry) -> anyhow::Result<()> {
				(**self).insert_scope_entry(entry).await
			}
		}

		#[async_trait]
		impl<C: TreeConfig, T: ScopeTreeWriteRepository<C> + ?Sized> ScopeTreeWriteRepository<C>
			for $ty
		{
			async fn store_scope_nodes(
				&mut self,
				scope: &ScopeId,
				nodes: Vec<(HasherOutput<C>, TreeNode<C>)>,
			) -> anyhow::Result<()> {
				(**self).store_scope_nodes(scope, nodes).await
			}
		}
	)+};
}

forward_scope_read!(&T, &mut T, Box<T>, Arc<T>);
forward_scope_write!(&mut T, Box<T>);

pub struct Nodes<R>(pub R);

impl<T: TreeConfig, R: ScopeTreeReadRepository<T>> ReadNodeStorage<T> for Nodes<R>
where
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
{
	type Error = StoreError;

	async fn get_node(&self, hash: &HasherOutput<T>) -> Result<TreeNode<T>, Self::Error> {
		self.0
			.scope_tree_node(hash)
			.await
			.and_then(|n| n.context("node was expected but wasn't received"))
			.map_err(StoreError)
	}
}

pub struct Scoped<'a, R>(pub &'a ScopeId, pub R);

impl<R: ScopeReadRepository> MMRStoreReadOps<CurrentHash> for Scoped<'_, R> {
	type Error = StoreError;

	async fn get_elem(&self, pos: u64) -> Result<Option<CurrentHash>, StoreError> {
		self.1.scope_log_node(self.0, pos).await.map_err(StoreError)
	}

	async fn get_elems(
		&self,
		positions: impl Iterator<Item = u64> + Send,
	) -> Result<Vec<Option<CurrentHash>>, StoreError> {
		self.1
			.scope_log_nodes(self.0, positions.collect())
			.await
			.map_err(StoreError)
	}
}

impl<R: ScopeWriteRepository> MMRStoreWriteOps<CurrentHash> for Scoped<'_, R> {
	type Error = StoreError;

	async fn append(&mut self, pos: u64, elems: Vec<CurrentHash>) -> Result<(), StoreError> {
		self.1
			.append_scope_log_nodes(self.0, pos, elems)
			.await
			.map_err(StoreError)
	}
}

impl<T: TreeConfig, R: ScopeTreeReadRepository<T>> ReadNodeStorage<T> for Scoped<'_, R>
where
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
{
	type Error = StoreError;

	async fn get_node(&self, hash: &HasherOutput<T>) -> Result<TreeNode<T>, Self::Error> {
		self.1
			.scope_tree_node(hash)
			.await
			.and_then(|n| n.context("node was expected but wasn't received"))
			.map_err(StoreError)
	}
}

impl<T: TreeConfig, R: ScopeTreeWriteRepository<T>> WriteNodeStorage<T> for Scoped<'_, R>
where
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
{
	async fn store_nodes(
		&mut self,
		nodes: impl Iterator<Item = (HasherOutput<T>, TreeNode<T>)> + Send,
	) -> Result<(), Self::Error> {
		self.1
			.store_scope_nodes(self.0, nodes.collect())
			.await
			.map_err(StoreError)
	}
}
