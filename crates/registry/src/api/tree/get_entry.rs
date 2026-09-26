use std::fmt::Debug;

use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::web;
use merkle_bplustree::MerkleBPlusTree;
use merkle_bplustree::TreeConfig;
use merkle_bplustree::hasher::Hasher;
use merkle_bplustree::proof::GetWithProof;
use pesde::source::pesde::registry::*;
use pesde_registry_core::features::tree::TreeReadRepository;
use serde::Serialize;

use crate::AppState;
use crate::api::tree::error::Error;
use crate::shared::tree::StoredTree;

super::tree_handler!("/entry/{key}", http_v2 => http_v2_impl);

async fn http_v2_impl<T: StoredTree>(
	app_state: web::Data<AppState>,
	path: web::Path<(CurrentHash, T::Key)>,
) -> Result<impl Responder, Error>
where
	T::Key: Debug + Serialize,
	T::Value: Debug + Serialize,
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
	T::Shaper: Default,
{
	let (root, key) = path.into_inner();
	let entry = handler(T::read_repo(app_state.db.as_ref()), &root, &key).await?;

	Ok(HttpResponse::Ok().json(entry))
}

async fn handler<T: TreeConfig>(
	db: &dyn TreeReadRepository<T>,
	root: &CurrentHash,
	key: &T::Key,
) -> Result<TreeEntryEndpointResponse<T>, Error>
where
	T::Key: Debug + Serialize,
	T::Value: Debug + Serialize,
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
	T::Shaper: Default,
{
	let node = db.get_node(root).await?.ok_or(Error::RootNotFound)?;
	let tree = MerkleBPlusTree::from_root(node, db, Default::default());
	match tree.get_with_proof(key).await {
		Ok(GetWithProof::Included(proof, value)) => {
			Ok(TreeEntryEndpointResponse::Included { proof, value })
		}
		Ok(GetWithProof::Excluded(proof)) => Ok(TreeEntryEndpointResponse::Excluded { proof }),
		Err(e) => Err(e.into()),
	}
}
