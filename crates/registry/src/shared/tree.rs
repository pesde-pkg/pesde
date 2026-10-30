use std::sync::Arc;

use pesde_core::merkle_bplustree::TreeConfig;
use pesde_core::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::db::ScopeWriteTransaction;
use pesde_registry_core::features::scope::ScopeTreeReadRepository;
use pesde_registry_core::features::scope::ScopeTreeWriteRepository;

pub trait ScopeTree: TreeConfig {
	fn root_from_state(state: &ScopeStateResponse) -> &CurrentHash;
	fn read_repo(db: &dyn Backend) -> &dyn ScopeTreeReadRepository<Self>;
	fn into_read_repo(db: Arc<dyn Backend>) -> Arc<dyn ScopeTreeReadRepository<Self>>;
	fn write_repo(tx: &mut dyn ScopeWriteTransaction) -> &mut dyn ScopeTreeWriteRepository<Self>;
	fn hash(&self) -> &CurrentHash;
	fn from_hash(hash: CurrentHash) -> Self;
}
impl ScopeTree for ScopeMembersTree {
	fn root_from_state(state: &ScopeStateResponse) -> &CurrentHash {
		&state.scope_members_root.0
	}
	fn read_repo(db: &dyn Backend) -> &dyn ScopeTreeReadRepository<Self> {
		db
	}
	fn into_read_repo(db: Arc<dyn Backend>) -> Arc<dyn ScopeTreeReadRepository<Self>> {
		db
	}
	fn write_repo(tx: &mut dyn ScopeWriteTransaction) -> &mut dyn ScopeTreeWriteRepository<Self> {
		tx
	}
	fn hash(&self) -> &CurrentHash {
		&self.0
	}
	fn from_hash(hash: CurrentHash) -> Self {
		Self(hash)
	}
}
impl ScopeTree for PackageVersionsTree {
	fn root_from_state(state: &ScopeStateResponse) -> &CurrentHash {
		&state.package_versions_root.0
	}
	fn read_repo(db: &dyn Backend) -> &dyn ScopeTreeReadRepository<Self> {
		db
	}
	fn into_read_repo(db: Arc<dyn Backend>) -> Arc<dyn ScopeTreeReadRepository<Self>> {
		db
	}
	fn write_repo(tx: &mut dyn ScopeWriteTransaction) -> &mut dyn ScopeTreeWriteRepository<Self> {
		tx
	}
	fn hash(&self) -> &CurrentHash {
		&self.0
	}
	fn from_hash(hash: CurrentHash) -> Self {
		Self(hash)
	}
}
impl ScopeTree for PackageDeprecationsTree {
	fn root_from_state(state: &ScopeStateResponse) -> &CurrentHash {
		&state.package_deprecations_root.0
	}
	fn read_repo(db: &dyn Backend) -> &dyn ScopeTreeReadRepository<Self> {
		db
	}
	fn into_read_repo(db: Arc<dyn Backend>) -> Arc<dyn ScopeTreeReadRepository<Self>> {
		db
	}
	fn write_repo(tx: &mut dyn ScopeWriteTransaction) -> &mut dyn ScopeTreeWriteRepository<Self> {
		tx
	}
	fn hash(&self) -> &CurrentHash {
		&self.0
	}
	fn from_hash(hash: CurrentHash) -> Self {
		Self(hash)
	}
}
