use std::num::NonZero;

use async_trait::async_trait;
use pesde::{hash::Hash, source::pesde::registry::*};

#[async_trait]
pub trait ScopeReadRepository {
	async fn scope_log_size(&self, scope: &ScopeId) -> anyhow::Result<Option<NonZero<u64>>>;

	async fn scope_log_entry(
		&self,
		scope: &ScopeId,
		pos: u64,
	) -> anyhow::Result<Option<ScopeEntry>>;

	async fn scope_state(
		&self,
		scope: &ScopeId,
		at_size: NonZero<u64>,
	) -> anyhow::Result<Option<ScopeStateResponse>>;
}

#[async_trait]
pub trait ScopeWriteRepository: ScopeReadRepository {
	async fn set_deprecation_plaintext(
		&mut self,
		hash: &Hash,
		plaintext: &str,
	) -> anyhow::Result<()>;

	async fn insert_entry(&mut self, entry: &ScopeEntry) -> anyhow::Result<()>;
}
