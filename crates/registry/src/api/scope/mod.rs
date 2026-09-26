use merkle_bplustree::{MerkleBPlusTree, hasher::Hasher};
use pesde::source::pesde::registry::{CurrentHash, ScopeStateResponse};
use pesde_registry_core::{db::ScopeWriteTransaction, features::tree::TreeWriteRepository};

use crate::shared::tree::StoredTree;

mod error;
mod get_entry;
mod get_head;
mod get_state;
mod post_entry;

pub(super) fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(get_head::http_v2)
		.service(get_entry::http_v2)
		.service(get_state::http_v2)
		.service(post_entry::http_v2);
}

async fn run_tree<T: StoredTree>(
	tx: &mut dyn ScopeWriteTransaction,
	state: &ScopeStateResponse,
	supplied_root: &T,
	cb: impl AsyncFnOnce(
		&mut MerkleBPlusTree<T, &mut dyn TreeWriteRepository<T>>,
	) -> Result<(), error::Error>,
) -> Result<(), error::Error>
where
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
	T::Shaper: Default,
{
	let mut tree = MerkleBPlusTree::from_root_hash(
		T::root_from_state(state),
		T::write_repo(tx),
		Default::default(),
	)
	.await?;
	cb(&mut tree).await?;
	if tree.root_hash() != *supplied_root.hash() {
		return Err(error::Error::ComputedRootDifferent);
	}
	Ok(())
}
