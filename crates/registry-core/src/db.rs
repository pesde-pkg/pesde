use std::num::NonZero;

use async_trait::async_trait;
use pesde::signature::PublicKey;
use pesde::source::pesde::registry::*;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct StoreError(pub anyhow::Error);

#[derive(Debug, Clone, Copy)]
#[must_use]
pub enum PermissionWidth<'a> {
	OnlySelf,
	Package(&'a LocalNameId),
	Owner,
}

#[must_use]
pub enum ExistingScopeLockResult {
	Ok {
		tx: Box<dyn ScopeWriteTransaction>,
		scope_size: NonZero<u64>,
	},
	Unauthorized,
	DoesntExist,
}

#[must_use]
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
	+ crate::features::global::GlobalReadRepository
	+ crate::features::scope::ScopeReadRepository
{
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
pub trait ScopeWriteTransaction: crate::features::scope::ScopeWriteRepository {
	async fn commit(self: Box<Self>) -> anyhow::Result<()>;
	async fn rollback(self: Box<Self>) -> anyhow::Result<()>;
}

#[async_trait]
pub trait GlobalWriteTransaction: crate::features::global::GlobalWriteRepository {
	fn into_scope_transaction(self: Box<Self>) -> Box<dyn ScopeWriteTransaction>;

	async fn rollback(self: Box<Self>) -> anyhow::Result<()>;
}
