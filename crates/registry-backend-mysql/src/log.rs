use anyhow::Context as _;
use async_trait::async_trait;
use futures::TryStreamExt as _;
use pesde_core::hash::Blake3Hash;
use pesde_core::registry::*;
use pesde_core::signature::KeyKind;
use pesde_core::signature::PublicKey;
use pesde_core::signature::Signature;
use pesde_core::signature::SignatureKind;
use pesde_registry_core::features::global::GlobalReadRepository;
use pesde_registry_core::features::global::GlobalWriteRepository;
use sqlx::QueryBuilder;
use sqlx::prelude::Type;
use sqlx::query;
use sqlx::types::Text;

use crate::AsExecutor;
use crate::MySqlBackend;
use crate::MySqlTransaction;
use crate::util::chrono_to_jiff;
use crate::util::insert_chunked;
use crate::util::jiff_to_chrono;

#[derive(Debug, Clone, Copy, Type)]
#[sqlx(rename_all = "snake_case")]
enum OpKind {
	ScopeGenesis,
}

async fn global_log_size(mut tx: impl AsExecutor) -> anyhow::Result<u64> {
	let record = query!("SELECT size FROM global_log")
		.fetch_one(tx.as_executor())
		.await?;

	Ok(record.size)
}

async fn global_log_entry(
	mut tx: impl AsExecutor,
	pos: u64,
) -> anyhow::Result<Option<GlobalEntry>> {
	let result = query!(
		"SELECT op_kind AS `op_kind: OpKind`, published_at FROM global_log_entry WHERE pos = ?",
		pos
	)
	.fetch_optional(tx.as_executor())
	.await?;

	let Some(record) = result else {
		return Ok(None);
	};

	let payload = match record.op_kind {
		OpKind::ScopeGenesis => GlobalEntryPayload::ScopeGenesis({
			let r = query!(
				r"
SELECT
    global_log_scope_genesis_entry.sig_algorithm AS `sig_algorithm: Text<SignatureKind>`,
    global_log_scope_genesis_entry.sig,
    global_log_scope_genesis_entry.first_entry_hash,
    global_log_scope_genesis_entry.name_blake3,
    public_key.algorithm AS `kind: Text<KeyKind>`,
	public_key.data
FROM global_log_scope_genesis_entry
INNER JOIN public_key ON public_key.id=global_log_scope_genesis_entry.owner_id
WHERE global_log_scope_genesis_entry.pos = ?",
				pos,
			)
			.fetch_one(tx.as_executor())
			.await?;

			Signed::new(UnvalidatedSigned {
				sig: Signature::new(r.sig_algorithm.0, r.sig).context("corrupt signature")?,
				body: ScopeGenesisPayload {
					first_entry_hash: r.first_entry_hash.parse()?,
					owner: PublicKey::new(r.kind.0, r.data).context("corrupt owner key")?,
					scope_id: ScopeId::new(
						Blake3Hash::new(r.name_blake3).context("corrupt genesis")?,
					),
				},
			})?
		}),
	};

	Ok(Some(Entry {
		pos,
		published_at: chrono_to_jiff(record.published_at)?,
		payload,
	}))
}

async fn global_log_node(mut tx: impl AsExecutor, pos: u64) -> anyhow::Result<Option<CurrentHash>> {
	let result = query!("SELECT blake3 FROM global_log_node WHERE pos = ?", pos)
		.fetch_optional(tx.as_executor())
		.await?;

	let Some(record) = result else {
		return Ok(None);
	};

	Ok(Some(
		Blake3Hash::new(record.blake3).context("corrupt node blake3")?,
	))
}

async fn global_log_nodes(
	mut tx: impl AsExecutor,
	positions: Vec<u64>,
) -> anyhow::Result<Vec<Option<CurrentHash>>> {
	let mut nodes = Vec::new();
	nodes.reserve_exact(positions.len());

	let mut query = QueryBuilder::new("SELECT blake3 FROM global_log_node WHERE pos IN (");
	let mut separated = query.separated(',');

	for pos in &positions {
		separated.push_bind(pos);
	}

	query.push(") ORDER BY FIELD(pos, ");

	let mut separated = query.separated(',');

	for pos in &positions {
		separated.push_bind(pos);
	}

	query.push(')');

	let mut query = query
		.build_query_as::<(Box<[u8]>,)>()
		.fetch(tx.as_executor());
	while let Some((bytes,)) = query.try_next().await? {
		nodes.push(Some(Blake3Hash::new(bytes)?));
	}

	Ok(nodes)
}

#[async_trait]
impl GlobalReadRepository for MySqlBackend {
	async fn global_log_size(&self) -> anyhow::Result<u64> {
		global_log_size(&self.pool).await
	}

	async fn global_log_entry(&self, pos: u64) -> anyhow::Result<Option<GlobalEntry>> {
		global_log_entry(&self.pool, pos).await
	}

	async fn global_log_node(&self, pos: u64) -> anyhow::Result<Option<CurrentHash>> {
		global_log_node(&self.pool, pos).await
	}

	async fn global_log_nodes(
		&self,
		positions: Vec<u64>,
	) -> anyhow::Result<Vec<Option<CurrentHash>>> {
		global_log_nodes(&self.pool, positions).await
	}
}

#[async_trait]
impl GlobalReadRepository for MySqlTransaction {
	async fn global_log_size(&self) -> anyhow::Result<u64> {
		let mut tx = self.tx.lock().await;
		global_log_size(&mut *tx).await
	}

	async fn global_log_entry(&self, pos: u64) -> anyhow::Result<Option<GlobalEntry>> {
		let mut tx = self.tx.lock().await;
		global_log_entry(&mut *tx, pos).await
	}

	async fn global_log_node(&self, pos: u64) -> anyhow::Result<Option<CurrentHash>> {
		let mut tx = self.tx.lock().await;
		global_log_node(&mut *tx, pos).await
	}

	async fn global_log_nodes(
		&self,
		positions: Vec<u64>,
	) -> anyhow::Result<Vec<Option<CurrentHash>>> {
		let mut tx = self.tx.lock().await;
		global_log_nodes(&mut *tx, positions).await
	}
}

#[async_trait]
impl GlobalWriteRepository for MySqlTransaction {
	async fn append_global_log_nodes(
		&mut self,
		pos: u64,
		elems: Vec<CurrentHash>,
	) -> anyhow::Result<()> {
		insert_chunked!(
			&mut **self.tx.get_mut(),
			"global_log_node",
			["pos", "blake3"],
			elems.iter().zip(pos..),
			|mut b, (hash, pos)| {
				b.push_bind(pos).push_bind(hash.0.as_slice());
			}
		)
		.await?;

		Ok(())
	}

	async fn set_global_log_size(&mut self, size: u64) -> anyhow::Result<()> {
		query!("UPDATE global_log SET size = ?", size)
			.execute(&mut **self.tx.get_mut())
			.await?;

		Ok(())
	}

	async fn insert_global_entry(&mut self, entry: &GlobalEntry) -> anyhow::Result<()> {
		let tx = self.tx.get_mut();
		let Entry {
			pos,
			published_at,
			payload,
		} = entry;

		let published_at = jiff_to_chrono(*published_at)?;

		match payload {
			GlobalEntryPayload::ScopeGenesis(payload) => {
				query!(
					"INSERT INTO global_log_entry (pos, op_kind, published_at) VALUES (?, ?, ?)",
					pos,
					OpKind::ScopeGenesis,
					published_at
				)
				.execute(&mut **tx)
				.await?;

				let UnvalidatedSigned {
					sig,
					body:
						ScopeGenesisPayload {
							scope_id,
							owner,
							first_entry_hash,
						},
				} = payload.inner();

				let owner_id = Self::insert_public_key(tx, owner).await?;

				query!(
					"INSERT INTO global_log_scope_genesis_entry (pos, sig_algorithm, sig, name_blake3, owner_id, first_entry_hash) VALUES (?, ?, ?, ?, ?, ?)",
					pos,
					Text(sig.kind()),
					sig.data(),
					scope_id.hash().0.as_slice(),
					owner_id,
					first_entry_hash.to_string(),
				)
				.execute(&mut **tx)
				.await?;
			}
		}

		Ok(())
	}
}
