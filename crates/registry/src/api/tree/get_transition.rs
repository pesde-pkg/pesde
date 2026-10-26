use std::collections::HashMap;
use std::fmt::Debug;

use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::web;
use merkle_bplustree::HasherOutput;
use merkle_bplustree::MerkleBPlusTree;
use merkle_bplustree::TreeConfig;
use merkle_bplustree::hasher::Hasher;
use merkle_bplustree::node::TreeNode;
use merkle_bplustree::proof::GetWithProof;
use merkle_bplustree::storage::ReadNodeStorage;
use merkle_bplustree::storage::WriteNodeStorage;
use pesde::bounded::BoundedVec;
use pesde::source::pesde::registry::*;
use pesde_registry_core::features::scope::Nodes;
use pesde_registry_core::features::scope::ScopeTreeReadRepository;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Db;
use crate::api::tree::error::Error;
use crate::shared::tree::ScopeTree;

super::tree_handler!((actix_web::http::Method::from_bytes(b"QUERY").unwrap()) "/transition", http_v2 => http_v2_impl);

async fn http_v2_impl<T: ScopeTree + Serialize + DeserializeOwned>(
	db: Db,
	path: web::Path<CurrentHash>,
	body: web::Json<BoundedVec<TreeOperation<T>, 4>>,
) -> Result<impl Responder, Error>
where
	T::Key: Debug + Serialize + DeserializeOwned,
	T::Value: Debug + Serialize + DeserializeOwned,
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
	T::Shaper: Default,
{
	let root = path.into_inner();
	let results = handler(T::read_repo(&**db), &root, body.into_inner().into_inner()).await?;

	Ok(HttpResponse::Ok().json(results))
}

async fn handler<T: ScopeTree + Serialize + DeserializeOwned>(
	db: &dyn ScopeTreeReadRepository<T>,
	root: &CurrentHash,
	operations: Vec<TreeOperation<T>>,
) -> Result<Vec<TreeTransitionResult<T>>, Error>
where
	T::Key: Debug + Serialize + DeserializeOwned,
	T::Value: Debug + Serialize + DeserializeOwned,
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
	T::Shaper: Default,
{
	let node = db.scope_tree_node(root).await?.ok_or(Error::RootNotFound)?;

	struct InMemoryWithFallbackStorage<C: TreeConfig, S: ReadNodeStorage<C>> {
		local: HashMap<HasherOutput<C>, TreeNode<C>>,
		fallback: S,
	}
	impl<C: TreeConfig, S: ReadNodeStorage<C> + Send + Sync> ReadNodeStorage<C>
		for InMemoryWithFallbackStorage<C, S>
	{
		type Error = S::Error;

		async fn get_node(&self, hash: &HasherOutput<C>) -> Result<TreeNode<C>, Self::Error> {
			if let Some(node) = self.local.get(hash).cloned() {
				return Ok(node);
			}

			self.fallback.get_node(hash).await
		}
	}
	impl<C: TreeConfig, S: ReadNodeStorage<C> + Send + Sync> WriteNodeStorage<C>
		for InMemoryWithFallbackStorage<C, S>
	{
		async fn store_nodes(
			&mut self,
			nodes: impl Iterator<Item = (HasherOutput<C>, TreeNode<C>)> + Send,
		) -> Result<(), Self::Error> {
			self.local.extend(nodes);
			Ok(())
		}
	}

	let mut tree = MerkleBPlusTree::from_root(
		node,
		InMemoryWithFallbackStorage {
			local: Default::default(),
			fallback: Nodes(db),
		},
		Default::default(),
	);

	let mut results = vec![];

	for op in operations {
		let proof = match op {
			TreeOperation::Modify { key, value } => {
				let (_, proof) = tree.insert_with_proof(key, value).await?;
				TreeTransitionProof::Modify(proof)
			}
			TreeOperation::Remove { key } => {
				let proof = tree.delete_with_proof(&key).await?;
				match proof {
					Some((_, proof)) => TreeTransitionProof::Delete(proof),
					None => TreeTransitionProof::DeleteNotApplied(
						match tree.get_with_proof_mapped(&key, |_| ()).await? {
							GetWithProof::Excluded(proof) => proof,
							GetWithProof::Included(_, _) => unreachable!(),
						},
					),
				}
			}
		};

		results.push(TreeTransitionResult {
			root: T::from_hash(tree.root_hash()),
			proof,
		});
	}

	Ok(results)
}
