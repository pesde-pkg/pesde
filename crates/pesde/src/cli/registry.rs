use std::array;
use std::fmt::Debug;
use std::fmt::Display;
use std::sync::Arc;

use anyhow::Context as _;
use async_stream::try_stream;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::stream::BoxStream;
use merkle_bplustree::TreeConfig;
use merkle_bplustree::hasher::Hasher;
use merkle_bplustree::proof::range::RangeVerifier as _;
use merkle_bplustree::proof::range::range_verifier;
use pesde::Url;
use pesde::bounded::BoundedSlice;
use pesde::hash::Hash;
use pesde::registry::*;
use reqwest::Method;
use reqwest::RequestBuilder;
use reqwest::Response;
use reqwest::header::AUTHORIZATION;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::AsyncBufReadExt as _;
use tokio::io::BufReader;
use tokio::sync::Semaphore;
use tokio_util::io::StreamReader;

use crate::cli::rootless_url;
use crate::cli::url_component_encode;

pub trait ClientTree: TreeConfig + Serialize + DeserializeOwned + Copy {
	const NAME: &'static str;

	fn root_hash(&self) -> &CurrentHash;
}

macro_rules! client_tree {
	($ty:ty, $name:literal) => {
		impl ClientTree for $ty {
			const NAME: &'static str = $name;

			fn root_hash(&self) -> &CurrentHash {
				&self.0
			}
		}
	};
}

client_tree!(ScopeMembersTree, "scope_members");
client_tree!(PackageVersionsTree, "package_versions");
client_tree!(PackageDeprecationsTree, "package_deprecations");

#[expect(dead_code)]
pub enum TreeUpdate<T: TreeConfig> {
	Insert { key: T::Key, value: T::Value },
	Update { key: T::Key, value: T::Value },
	Set { key: T::Key, value: T::Value },
	Remove { key: T::Key },
	RemoveIfPresent { key: T::Key },
}

#[derive(Debug)]
pub struct RegistryClient {
	url: Url,
	reqwest: reqwest::Client,
	network_concurrency: Arc<Semaphore>,
	authorization: Option<Box<str>>,
}

impl RegistryClient {
	pub fn new(
		url: Url,
		reqwest: reqwest::Client,
		network_concurrency: Arc<Semaphore>,
		authorization: Option<Box<str>>,
	) -> Self {
		Self {
			url,
			reqwest,
			network_concurrency,
			authorization,
		}
	}

	fn request(&self, method: Method, path: impl Display) -> RequestBuilder {
		let mut request = self
			.reqwest
			.request(method, format!("{}/api/v2/{path}", rootless_url(&self.url)));

		if let Some(authorization) = &self.authorization {
			request = request.header(AUTHORIZATION, authorization.as_ref());
		}

		request
	}

	async fn fetch(&self, request: RequestBuilder) -> anyhow::Result<Response> {
		let _permit = self.network_concurrency.acquire().await;

		request
			.send()
			.await
			.context("failed to send request to registry")?
			.error_for_status()
			.context("registry returned an error response")
	}

	pub async fn scope_head(
		&self,
		scope_id: &ScopeId,
	) -> anyhow::Result<LogHeadResponse<ScopeEntryPayload>> {
		self.fetch(self.request(
			Method::GET,
			format!(
				"scope/{}/log/head",
				url_component_encode(&scope_id.to_string())
			),
		))
		.await?
		.json()
		.await
		.context("failed to parse response from registry")
	}

	pub async fn post_scope_entry(
		&self,
		payload: &ScopeEntryPayload,
	) -> anyhow::Result<Entry<ScopeEntryPayload>> {
		self.fetch(self.request(Method::POST, "scope/log/entry").json(payload))
			.await?
			.json()
			.await
			.context("failed to parse response from registry")
	}

	pub async fn tree_entry<T: ClientTree>(
		&self,
		root: &T,
		key: &T::Key,
	) -> anyhow::Result<Option<T::Value>>
	where
		T::Key: Debug + Display + DeserializeOwned,
		T::Value: Debug + DeserializeOwned,
		T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
	{
		let response: TreeEntryEndpointResponse<T> = self
			.fetch(self.request(
				Method::GET,
				format!(
					"tree/{}/{}/entry/{}",
					T::NAME,
					url_component_encode(&root.root_hash().to_string()),
					url_component_encode(&key.to_string()),
				),
			))
			.await?
			.json()
			.await
			.context("failed to parse response from registry")?;

		match response {
			TreeEntryEndpointResponse::Included { proof, value } => {
				if proof.verify(root.root_hash(), key, &value).is_err() {
					anyhow::bail!("failed to verify the {} tree proof", T::NAME);
				}

				Ok(Some(value))
			}
			TreeEntryEndpointResponse::Excluded { proof } => {
				if proof.verify(root.root_hash(), key).is_err() {
					anyhow::bail!("failed to verify the {} tree proof", T::NAME);
				}

				Ok(None)
			}
		}
	}

	pub async fn tree_entries<T: ClientTree + Send + 'static>(
		&self,
		root: &T,
	) -> anyhow::Result<BoxStream<'static, anyhow::Result<(T::Key, T::Value)>>>
	where
		T::Key: Debug + Send + DeserializeOwned + 'static,
		T::Value: Debug + Send + DeserializeOwned + 'static,
		T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
	{
		let root = *root;

		let response = self
			.fetch(self.request(
				Method::GET,
				format!(
					"tree/{}/{}/entries",
					T::NAME,
					url_component_encode(&root.root_hash().to_string()),
				),
			))
			.await?;

		let stream = try_stream! {
			let reader = StreamReader::new(response.bytes_stream().map_err(std::io::Error::other));
			let mut lines = BufReader::new(reader).lines();

			let mut verifier = range_verifier::<T, T::Key, _>(..);

			while let Some(line) = lines.next_line().await? {
				match serde_json::from_str::<TreeEntriesEndpointResponse<T>>(&line)? {
					TreeEntriesEndpointResponse::Entry { key, value } => {
						if verifier.push_entry(&key, &value).is_err() {
							Err(anyhow::anyhow!("failed to push an entry to the {} tree proof verifier", T::NAME))?;
						}

						yield (key, value);
					}
					TreeEntriesEndpointResponse::Node(node) => {
						if verifier.push_node(node).is_err() {
							Err(anyhow::anyhow!("failed to push a node to the {} tree proof verifier", T::NAME))?;
						}
					}
				}
			}

			if verifier.finish(root.root_hash()).is_err() {
				Err(anyhow::anyhow!("failed to verify the {} tree proof", T::NAME))?;
			}
		};

		Ok(stream.boxed())
	}

	pub async fn tree_transition<T: ClientTree, const N: usize>(
		&self,
		root: &T,
		updates: [TreeUpdate<T>; N],
	) -> anyhow::Result<T>
	where
		T::Key: Debug + Clone + Serialize + DeserializeOwned,
		T::Value: Debug + Clone + Serialize + DeserializeOwned,
		T::Hasher: Hasher<T::Key, T::Value, Output = CurrentHash>,
		T::Shaper: Default,
	{
		let operations: [TreeOperation<T>; N] = array::from_fn(|i| match &updates[i] {
			TreeUpdate::Insert { key, value }
			| TreeUpdate::Update { key, value }
			| TreeUpdate::Set { key, value } => TreeOperation::Modify {
				key: key.clone(),
				value: value.clone(),
			},
			TreeUpdate::Remove { key } | TreeUpdate::RemoveIfPresent { key } => {
				TreeOperation::Remove { key: key.clone() }
			}
		});

		let results = self
			.fetch(
				self.request(
					Method::QUERY,
					format!(
						"tree/{}/{}/transition",
						T::NAME,
						url_component_encode(&root.root_hash().to_string()),
					),
				)
				.json(&operations[..]),
			)
			.await?
			.json::<BoundedSlice<TreeTransitionResult<T>, N>>()
			.await
			.context("failed to parse response from registry")?
			.into_inner();
		anyhow::ensure!(
			results.len() == N,
			"registry returned an unexpected amount of results"
		);

		let shaper = T::Shaper::default();
		let mut current = *root;

		for (update, result) in updates.iter().zip(results) {
			match (update, result.operation) {
				(
					TreeUpdate::Insert { key, value }
					| TreeUpdate::Update { key, value }
					| TreeUpdate::Set { key, value },
					TreeTransitionOperationResult::Modified(proof),
				) => {
					let Ok(inserted) = proof.verify(
						current.root_hash(),
						result.root.root_hash(),
						key,
						value,
						&shaper,
					) else {
						anyhow::bail!("failed to verify proof of tree transition from registry");
					};

					match update {
						TreeUpdate::Insert { .. } if !inserted => {
							anyhow::bail!("the entry already exists in the {} tree", T::NAME)
						}
						TreeUpdate::Update { .. } if inserted => {
							anyhow::bail!("the entry doesn't exist in the {} tree", T::NAME)
						}
						_ => {}
					}
				}
				(
					TreeUpdate::Remove { key } | TreeUpdate::RemoveIfPresent { key },
					TreeTransitionOperationResult::Removed { proof, value_hash },
				) => {
					if proof
						.verify_hash(
							current.root_hash(),
							result.root.root_hash(),
							key,
							&value_hash,
							&shaper,
						)
						.is_err()
					{
						anyhow::bail!("failed to verify proof of tree transition from registry");
					}
				}
				(
					TreeUpdate::Remove { key } | TreeUpdate::RemoveIfPresent { key },
					TreeTransitionOperationResult::NotRemoved(proof),
				) => {
					if proof.verify(current.root_hash(), key).is_err()
						|| result.root.root_hash() != current.root_hash()
					{
						anyhow::bail!("failed to verify proof of tree transition from registry");
					}

					if matches!(update, TreeUpdate::Remove { .. }) {
						anyhow::bail!("the entry doesn't exist in the {} tree", T::NAME);
					}
				}
				_ => anyhow::bail!("registry returned an unexpected proof type"),
			}

			current = result.root;
		}

		Ok(current)
	}

	pub async fn blob(&self, hash: &Hash) -> anyhow::Result<bytes::Bytes> {
		let bytes = self
			.fetch(self.request(
				Method::GET,
				format!(
					"blob/by-hash/{}/{}",
					url_component_encode(&hash.algorithm().to_string()),
					url_component_encode(&hash.encoded()),
				),
			))
			.await?
			.bytes()
			.await
			.context("failed to download blob")?;

		let actual = Hash::digest(hash.algorithm(), &bytes);
		anyhow::ensure!(
			actual == *hash,
			"blob hash mismatch: expected `{hash}`, got `{actual}`"
		);

		Ok(bytes)
	}
}
