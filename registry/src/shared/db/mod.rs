use std::any::Any;

use async_trait::async_trait;
use merkleberg::MMRIVER;
use merkleberg::MMRStoreReadOps;
use merkleberg::MMRStoreWriteOps;
use pesde::hash::RawHash;
use pesde::names::Name;
use pesde::names::Scope;
use pesde::signature::PublicKey;
use pesde::source::pesde::registry::*;
use serde::Serialize;
use sqlx::Database as _;
use sqlx::MySql;
use sqlx::prelude::Type;

pub mod mysql;

pub async fn connect(url: &str) -> Box<dyn Backend> {
	let protocol = url.split_once(':').map_or("", |(protocol, _)| protocol);

	if MySql::URL_SCHEMES.contains(&protocol) {
		return Box::new(mysql::MySqlBackend::connect(url).await);
	}

	panic!("unsupported database protocol `{protocol}`")
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct StoreError(#[source] pub anyhow::Error);

#[derive(Debug, thiserror::Error)]
pub enum PackageWriteError {
	#[error("the package version has already been published")]
	VersionAlreadyExists,

	#[error("the package version does not exist")]
	UnknownPackageVersion,

	#[error("the package version is already yanked")]
	AlreadyYanked,

	#[error("the package version is not yanked")]
	NotYanked,

	#[error("the package is already deprecated")]
	AlreadyDeprecated,

	#[error("the package is not deprecated")]
	NotDeprecated,

	#[error(transparent)]
	Internal(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityWriteError {
	#[error("the identity id has already been registered")]
	NonUniqueIdentityId,

	#[error("the public key has already been registered")]
	NonUniquePublicKey,

	#[error(transparent)]
	Internal(#[from] anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
	#[error("identity `{0}` is mentioned but not registered")]
	UnregisteredIdentity(IdentityId),

	#[error(transparent)]
	Internal(#[from] anyhow::Error),
}

#[derive(Debug, Type)]
#[sqlx(rename_all = "snake_case")]
pub enum EntryKind {
	Scope,
	RegisterIdentity,
	IdentityRotation,
	AdminScopeTransfer,
}

#[async_trait]
pub trait ReadStore: Send + Sync {
	async fn get_node(&self, pos: u64) -> Result<Option<RawHash>, StoreError>;

	async fn get_nodes(&self, positions: Vec<u64>) -> Result<Vec<Option<RawHash>>, StoreError> {
		let mut nodes = Vec::with_capacity(positions.len());
		for pos in positions {
			nodes.push(self.get_node(pos).await?);
		}
		Ok(nodes)
	}
}

impl MMRStoreReadOps<RawHash> for Box<dyn ReadStore> {
	type Error = StoreError;

	async fn get_elem(&self, pos: u64) -> Result<Option<RawHash>, StoreError> {
		self.get_node(pos).await
	}

	async fn get_elems(
		&self,
		positions: impl Iterator<Item = u64> + Send,
	) -> Result<Vec<Option<RawHash>>, StoreError> {
		self.get_nodes(positions.collect()).await
	}
}

#[async_trait]
pub trait WriteStore: Send + Sync + Any {
	async fn get_node(&self, pos: u64) -> Result<Option<RawHash>, StoreError>;

	async fn get_nodes(&self, positions: Vec<u64>) -> Result<Vec<Option<RawHash>>, StoreError> {
		let mut nodes = Vec::with_capacity(positions.len());
		for pos in positions {
			nodes.push(self.get_node(pos).await?);
		}
		Ok(nodes)
	}

	async fn append_nodes(&mut self, pos: u64, elems: Vec<RawHash>) -> Result<(), StoreError>;
	async fn set_size(&mut self, size: u64) -> anyhow::Result<()>;
	async fn commit(self: Box<Self>) -> anyhow::Result<()>;
}

impl MMRStoreReadOps<RawHash> for Box<dyn WriteStore> {
	type Error = StoreError;

	async fn get_elem(&self, pos: u64) -> Result<Option<RawHash>, StoreError> {
		self.get_node(pos).await
	}

	async fn get_elems(
		&self,
		positions: impl Iterator<Item = u64> + Send,
	) -> Result<Vec<Option<RawHash>>, StoreError> {
		self.get_nodes(positions.collect()).await
	}
}

impl MMRStoreWriteOps<RawHash> for Box<dyn WriteStore> {
	type Error = StoreError;

	async fn append(&mut self, pos: u64, elems: Vec<RawHash>) -> Result<(), StoreError> {
		self.append_nodes(pos, elems).await
	}
}

pub enum ScopeControl<'a> {
	Write(&'a Name),
	PublishOrCreate(&'a Name),
	Owner,
}

pub struct ScopeAccess {
	pub pos: u64,
	pub scope_exists: bool,
}

pub struct AuthorKey {
	pub identity: IdentityId,
	pub key: PublicKey,
}

pub async fn append_leaf(
	store: Box<dyn WriteStore>,
	pos: u64,
	body: &impl Serialize,
) -> anyhow::Result<(Box<dyn WriteStore>, u64)> {
	let mut mmr: MMRIVER<CurrentMmrMerge, Box<dyn WriteStore>> = MMRIVER::new(pos, store);
	mmr.push(&canonical_bytes(body)).await?;
	mmr.commit().await?;

	let next_pos = mmr.mmr_size();
	let mut store = mmr.into_store();
	store.set_size(next_pos).await?;
	Ok((store, next_pos))
}

#[async_trait]
pub trait Backend:
	Send
	+ Sync
	+ crate::features::package::Repository
	+ crate::features::identity::Repository
	+ crate::features::scope::Repository
	+ crate::features::log::Repository
	+ crate::features::search::Repository
{
	async fn current_size(&self) -> anyhow::Result<u64>;

	async fn read_mmr_at(
		&self,
		size: u64,
	) -> anyhow::Result<MMRIVER<CurrentMmrMerge, Box<dyn ReadStore>>>;

	async fn read_mmr(&self) -> anyhow::Result<MMRIVER<CurrentMmrMerge, Box<dyn ReadStore>>> {
		self.read_mmr_at(self.current_size().await?).await
	}

	async fn begin_write(&self) -> anyhow::Result<Box<dyn WriteStore>>;

	async fn current_identity_key(
		&self,
		store: &mut Box<dyn WriteStore>,
		id: &IdentityId,
	) -> anyhow::Result<Option<PublicKey>>;

	async fn lock_tree(&self, store: &mut Box<dyn WriteStore>) -> anyhow::Result<u64>;

	async fn scope_write_access(
		&self,
		store: &mut Box<dyn WriteStore>,
		scope: &Scope,
		identity: &IdentityId,
		control: ScopeControl<'_>,
	) -> anyhow::Result<Option<ScopeAccess>>;

	async fn author_key(
		&self,
		store: &mut Box<dyn WriteStore>,
		id: &IdentityId,
	) -> anyhow::Result<Option<AuthorKey>> {
		Ok(self
			.current_identity_key(store, id)
			.await?
			.map(|key| AuthorKey { identity: *id, key }))
	}

	async fn lock_tree_as(
		&self,
		store: &mut Box<dyn WriteStore>,
		author: &AuthorKey,
	) -> anyhow::Result<Option<u64>> {
		let pos = self.lock_tree(store).await?;
		Ok(self
			.author_key(store, &author.identity)
			.await?
			.is_some_and(|new| author.key == new.key)
			.then_some(pos))
	}

	async fn authorize_scope_write(
		&self,
		store: &mut Box<dyn WriteStore>,
		scope: &Scope,
		author: &AuthorKey,
		control: ScopeControl<'_>,
	) -> anyhow::Result<Option<ScopeAccess>> {
		let Some(access) = self
			.scope_write_access(store, scope, &author.identity, control)
			.await?
		else {
			return Ok(None);
		};
		Ok(self
			.author_key(store, &author.identity)
			.await?
			.is_some_and(|new| author.key == new.key)
			.then_some(access))
	}
}
