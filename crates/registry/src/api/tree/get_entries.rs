use std::fmt::Debug;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::Responder;
use actix_web::web;
use actix_web_lab::respond::NdJson;
use futures::Stream;
use futures::TryStreamExt as _;
use merkle_bplustree::MerkleBPlusTree;
use merkle_bplustree::TreeConfig;
use merkle_bplustree::hasher::Hasher;
use merkle_bplustree::proof::range::GetRangeProof;
use pesde::source::pesde::registry::*;
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
	Ok(EntriesStream {
		stream: Some(tree.into_proven_range(..)),
		phantom: PhantomData,
	}
	.map_err(Into::into))
}

pin_project_lite::pin_project! {
	struct EntriesStream<T, S> {
		#[pin]
		stream: Option<S>,
		phantom: PhantomData<T>
	}
}

impl<T: TreeConfig, E, S: Stream<Item = Result<(T::Key, T::Value), E>> + GetRangeProof<T>> Stream
	for EntriesStream<T, S>
where
	T::Key: Debug,
	T::Value: Debug,
	T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
{
	type Item = Result<TreeEntriesEndpointResponse<T>, E>;

	fn poll_next(
		self: Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Option<Self::Item>> {
		let mut this = self.project();
		let Some(mut stream) = this.stream.as_mut().as_pin_mut() else {
			return std::task::Poll::Ready(None);
		};

		if let Some(i) = std::task::ready!(stream.as_mut().poll_next(cx)) {
			let i = i.map(|(key, value)| TreeEntriesEndpointResponse::Entry { key, value });

			return std::task::Poll::Ready(Some(i));
		}

		let proof = stream.get_proof();
		this.stream.set(None);

		std::task::Poll::Ready(Some(Ok(TreeEntriesEndpointResponse::Proof { proof })))
	}
}
