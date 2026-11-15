use std::fmt::Debug;
use std::sync::Arc;

use actix_web::Responder;
use actix_web::web;
use actix_web_lab::respond::NdJson;
use futures::Stream;
use futures::TryStreamExt as _;
use pesde::merkle_bplustree::MerkleBPlusTree;
use pesde::merkle_bplustree::TreeConfig;
use pesde::merkle_bplustree::hasher::Hasher;
use pesde::merkle_bplustree::proof::range::RangeItem;
use pesde::registry::*;
use pesde_registry_core::features::scope::Nodes;
use pesde_registry_core::features::scope::ScopeTreeReadRepository;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Db;
use crate::api::tree::error::Error;
use crate::shared::tree::ScopeTree;

super::tree_handler!((actix_web::http::Method::GET) "/entries", http_v2 => http_v2_impl);

async fn http_v2_impl<T: ScopeTree>(
	db: Db,
	path: web::Path<CurrentHash>,
) -> Result<impl Responder, Error>
where
	T::Key: Debug + Serialize + DeserializeOwned,
	T::Value: Debug + Serialize + DeserializeOwned,
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
	T::Shaper: Default,
{
	let root = path.into_inner();
	let entries = handler(T::into_read_repo(db.into_inner()), &root).await?;

	Ok(NdJson::new(entries).into_responder())
}

async fn handler<T: TreeConfig>(
	db: Arc<dyn ScopeTreeReadRepository<T>>,
	root: &CurrentHash,
) -> Result<impl Stream<Item = Result<TreeEntriesEndpointResponse<T>, Error>> + use<T>, Error>
where
	T::Key: Debug + Serialize + DeserializeOwned,
	T::Value: Debug + Serialize + DeserializeOwned,
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
	T::Shaper: Default,
{
	let node = db.scope_tree_node(root).await?.ok_or(Error::RootNotFound)?;
	let tree = MerkleBPlusTree::from_root(node, Nodes(db), Default::default());
	Ok(tree
		.into_proven_range(..)
		.map_ok(|item| match item {
			RangeItem::Entry((key, value)) => TreeEntriesEndpointResponse::Entry { key, value },
			RangeItem::Node(node) => TreeEntriesEndpointResponse::Node(node),
		})
		.map_err(Into::into))
}
