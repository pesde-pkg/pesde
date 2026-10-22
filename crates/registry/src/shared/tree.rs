use std::sync::Arc;

use merkle_bplustree::TreeConfig;
use pesde::source::pesde::registry::*;
use pesde_registry_core::{
	db::{Backend, ScopeWriteTransaction},
	features::tree::{TreeReadRepository, TreeWriteRepository},
};

pub trait StoredTree: TreeConfig {
	fn root_from_state(state: &ScopeStateResponse) -> &CurrentHash;
	fn read_repo(db: &dyn Backend) -> &dyn TreeReadRepository<Self>;
	fn into_read_repo(db: Arc<dyn Backend>) -> Arc<dyn TreeReadRepository<Self>>;
	fn write_repo(tx: &mut dyn ScopeWriteTransaction) -> &mut dyn TreeWriteRepository<Self>;
	fn hash(&self) -> &CurrentHash;
	fn from_hash(hash: CurrentHash) -> Self;
}
impl StoredTree for ScopeMembersTree {
	fn root_from_state(state: &ScopeStateResponse) -> &CurrentHash {
		&state.scope_members_root.0
	}
	fn read_repo(db: &dyn Backend) -> &dyn TreeReadRepository<Self> {
		db
	}
	fn into_read_repo(db: Arc<dyn Backend>) -> Arc<dyn TreeReadRepository<Self>> {
		db
	}
	fn write_repo(tx: &mut dyn ScopeWriteTransaction) -> &mut dyn TreeWriteRepository<Self> {
		tx
	}
	fn hash(&self) -> &CurrentHash {
		&self.0
	}
	fn from_hash(hash: CurrentHash) -> Self {
		Self(hash)
	}
}
impl StoredTree for PackageVersionsTree {
	fn root_from_state(state: &ScopeStateResponse) -> &CurrentHash {
		&state.package_versions_root.0
	}
	fn read_repo(db: &dyn Backend) -> &dyn TreeReadRepository<Self> {
		db
	}
	fn into_read_repo(db: Arc<dyn Backend>) -> Arc<dyn TreeReadRepository<Self>> {
		db
	}
	fn write_repo(tx: &mut dyn ScopeWriteTransaction) -> &mut dyn TreeWriteRepository<Self> {
		tx
	}
	fn hash(&self) -> &CurrentHash {
		&self.0
	}
	fn from_hash(hash: CurrentHash) -> Self {
		Self(hash)
	}
}
impl StoredTree for PackageDeprecationsTree {
	fn root_from_state(state: &ScopeStateResponse) -> &CurrentHash {
		&state.package_deprecations_root.0
	}
	fn read_repo(db: &dyn Backend) -> &dyn TreeReadRepository<Self> {
		db
	}
	fn into_read_repo(db: Arc<dyn Backend>) -> Arc<dyn TreeReadRepository<Self>> {
		db
	}
	fn write_repo(tx: &mut dyn ScopeWriteTransaction) -> &mut dyn TreeWriteRepository<Self> {
		tx
	}
	fn hash(&self) -> &CurrentHash {
		&self.0
	}
	fn from_hash(hash: CurrentHash) -> Self {
		Self(hash)
	}
}
