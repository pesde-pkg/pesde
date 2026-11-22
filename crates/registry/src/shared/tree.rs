use std::sync::Arc;

use pesde::merkle_bplustree::MerkleBPlusTree;
use pesde::merkle_bplustree::TreeConfig;
use pesde::merkle_bplustree::hasher::Hasher;
use pesde::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::db::ScopeWriteTransaction;
use pesde_registry_core::db::StoreError;
use pesde_registry_core::features::scope::ScopeTreeReadRepository;
use pesde_registry_core::features::scope::ScopeTreeWriteRepository;
use pesde_registry_core::features::scope::Scoped;

pub trait FromTreeError: From<StoreError> {
	fn root_mismatch() -> Self;
}

pub async fn run_tree<T: ScopeTree, E: FromTreeError>(
	scope: &ScopeId,
	tx: &mut dyn ScopeWriteTransaction,
	state: &mut ScopeState,
	supplied_root: &T,
	cb: impl AsyncFnOnce(
		&mut MerkleBPlusTree<T, Scoped<'_, &mut dyn ScopeTreeWriteRepository<T>>>,
	) -> Result<(), E>,
) -> Result<(), E>
where
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
	T::Shaper: Default,
{
	let root = T::root_from_state(state);
	let mut tree = MerkleBPlusTree::from_root_hash(
		&*root,
		Scoped(scope, T::write_repo(tx)),
		Default::default(),
	)
	.await?;
	cb(&mut tree).await?;
	if tree.root_hash() != *supplied_root.hash() {
		return Err(E::root_mismatch());
	}
	*root = *supplied_root.hash();
	Ok(())
}

pub trait ScopeTree: TreeConfig {
	fn root_from_state(state: &mut ScopeState) -> &mut CurrentHash;
	fn read_repo(db: &dyn Backend) -> &dyn ScopeTreeReadRepository<Self>;
	fn into_read_repo(db: Arc<dyn Backend>) -> Arc<dyn ScopeTreeReadRepository<Self>>;
	fn write_repo(tx: &mut dyn ScopeWriteTransaction) -> &mut dyn ScopeTreeWriteRepository<Self>;
	fn hash(&self) -> &CurrentHash;
	fn from_hash(hash: CurrentHash) -> Self;
}
impl ScopeTree for ScopeMembersTree {
	fn root_from_state(state: &mut ScopeState) -> &mut CurrentHash {
		&mut state.scope_members_root.0
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
	fn root_from_state(state: &mut ScopeState) -> &mut CurrentHash {
		&mut state.package_versions_root.0
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
	fn root_from_state(state: &mut ScopeState) -> &mut CurrentHash {
		&mut state.package_deprecations_root.0
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
