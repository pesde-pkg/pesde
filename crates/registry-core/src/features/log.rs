use async_trait::async_trait;
use pesde::names::Scope;
use pesde::source::pesde::registry::*;

#[async_trait]
pub trait Repository {
	async fn global_size(&self) -> anyhow::Result<u64>;

	async fn global_entry(&self, pos: u64) -> anyhow::Result<Option<GenesisEntry>>;

	async fn scope_size(&self, scope: &Scope) -> anyhow::Result<u64>;

	async fn scope_entry(&self, scope: &Scope, pos: u64) -> anyhow::Result<Option<ScopeEntry>>;
}
