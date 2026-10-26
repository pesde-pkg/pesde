use std::num::NonZero;
use std::sync::Arc;

use async_trait::async_trait;
use futures::lock::Mutex;
use pesde::{
	hash::{Blake3Hash, Hasher as _},
	signature::PublicKey,
	source::pesde::registry::*,
};
use pesde_registry_core::db::{
	Backend, CreatingScopeLockResult, ExistingScopeLockResult, GlobalWriteTransaction,
	PermissionWidth, ScopeWriteTransaction,
};
use sqlx::{
	Database as _, MySql, MySqlExecutor, MySqlPool, mysql::MySqlPoolOptions, query, types::Text,
};

use crate::util::insert_chunked;

mod log;
mod scope;
mod util;

pub const URL_SCHEMES: &[&str] = MySql::URL_SCHEMES;

pub async fn connect(url: &str) -> anyhow::Result<Arc<dyn Backend>> {
	let pool = MySqlPoolOptions::new().connect(url).await?;

	sqlx::migrate!().run(&pool).await?;

	Ok(Arc::new(MySqlBackend { pool }))
}

struct MySqlBackend {
	pool: MySqlPool,
}

struct MySqlTransaction {
	tx: Mutex<sqlx::MySqlTransaction<'static>>,
}

#[async_trait]
impl Backend for MySqlBackend {
	async fn begin_write_existing(
		&self,
		scope_id: &ScopeId,
		user: &PublicKey,
		permission_width: PermissionWidth<'_>,
	) -> anyhow::Result<ExistingScopeLockResult> {
		// query before locking to avoid letting arbitrary requests lock the log
		// this is not exploitable by race condition as this is rechecked after locking
		async fn check(
			scope_id: &ScopeId,
			user: &PublicKey,
			permission_width: PermissionWidth<'_>,
			mut tx: impl AsExecutor,
		) -> anyhow::Result<Option<ExistingScopeLockResult>> {
			todo!();
			let record = query!(
				"SELECT EXISTS(SELECT 1 FROM global_log_scope_genesis_entry WHERE name_blake3 = ?) AS `exists: bool`",
				scope_id.hash().0.as_slice()
			)
			.fetch_one(tx.as_executor())
			.await?;

			if !record.exists {
				return Ok(Some(ExistingScopeLockResult::DoesntExist));
			}

			Ok(None)
		}

		if let Some(ret) = check(scope_id, user, permission_width, &self.pool).await? {
			return Ok(ret);
		}

		let mut tx = self.pool.begin().await?;
		let scope_log = query!(
			r"
SELECT scope.log_size AS `log_size: NonZero<u64>`
FROM scope
INNER JOIN global_log_scope_genesis_entry ON global_log_scope_genesis_entry.pos=scope.genesis_pos
WHERE global_log_scope_genesis_entry.name_blake3 = ?
FOR UPDATE",
			scope_id.hash().0.as_slice()
		)
		.fetch_one(&mut *tx)
		.await?;

		if let Some(ret) = check(scope_id, user, permission_width, &mut tx).await? {
			return Ok(ret);
		}

		Ok(ExistingScopeLockResult::Ok {
			tx: Box::new(MySqlTransaction { tx: Mutex::new(tx) }),
			scope_size: scope_log.log_size,
		})
	}

	async fn begin_write_creating(
		&self,
		scope_id: &ScopeId,
	) -> anyhow::Result<CreatingScopeLockResult> {
		// query before locking to avoid letting arbitrary requests lock the log
		// this is not exploitable by race condition as this is rechecked after locking
		async fn check(
			scope_id: &ScopeId,
			mut tx: impl AsExecutor,
		) -> anyhow::Result<Option<CreatingScopeLockResult>> {
			let record = query!(
				"SELECT EXISTS(SELECT 1 FROM global_log_scope_genesis_entry WHERE name_blake3 = ?) AS `exists: bool`",
				scope_id.hash().0.as_slice()
			)
			.fetch_one(tx.as_executor())
			.await?;

			if record.exists {
				return Ok(Some(CreatingScopeLockResult::AlreadyExists));
			}

			Ok(None)
		}

		if let Some(ret) = check(scope_id, &self.pool).await? {
			return Ok(ret);
		}

		let mut tx = self.pool.begin().await?;
		let global_log = query!("SELECT size FROM global_log FOR UPDATE")
			.fetch_one(&mut *tx)
			.await?;

		if let Some(ret) = check(scope_id, &mut tx).await? {
			return Ok(ret);
		}

		Ok(CreatingScopeLockResult::Ok {
			tx: Box::new(MySqlTransaction { tx: Mutex::new(tx) }),
			global_size: global_log.size,
		})
	}
}

#[async_trait]
impl ScopeWriteTransaction for MySqlTransaction {
	async fn commit(self: Box<Self>) -> anyhow::Result<()> {
		self.tx.into_inner().commit().await.map_err(Into::into)
	}

	async fn rollback(self: Box<Self>) -> anyhow::Result<()> {
		self.tx.into_inner().rollback().await.map_err(Into::into)
	}
}

#[async_trait]
impl GlobalWriteTransaction for MySqlTransaction {
	fn into_scope_transaction(self: Box<Self>) -> Box<dyn ScopeWriteTransaction> {
		self
	}

	async fn rollback(self: Box<Self>) -> anyhow::Result<()> {
		self.tx.into_inner().rollback().await.map_err(Into::into)
	}
}

impl MySqlTransaction {
	pub async fn insert_public_key(
		tx: &mut sqlx::MySqlTransaction<'_>,
		public_key: &PublicKey,
	) -> anyhow::Result<u64> {
		let blake3 = Blake3Hash::digest(public_key.data());

		let result = query!(
			"INSERT INTO public_key (id, algorithm, data, blake3) VALUES (NULL, ?, ?, ?) ON DUPLICATE KEY UPDATE id = LAST_INSERT_ID(id)",
			Text(public_key.kind()),
			public_key.data(),
			blake3.0.as_slice(),
		).execute(&mut **tx).await?;

		Ok(result.last_insert_id())
	}

	pub async fn insert_grant(
		tx: &mut sqlx::MySqlTransaction<'_>,
		grant: &ScopeGrant,
	) -> anyhow::Result<Blake3Hash> {
		let mut blake3 = Blake3Hash::hasher();
		for entry in grant.0.iter() {
			blake3.update(entry.pkg().hash().0.as_slice());
		}
		let blake3 = blake3.finalize();

		let result = query!(
			"INSERT INTO member_grant (blake3) VALUES (?)",
			blake3.0.as_slice()
		)
		.execute(&mut **tx)
		.await;

		match result {
			Ok(_) => {
				insert_chunked!(
					&mut **tx,
					"member_grant_package",
					["grant_blake3", "local_name_blake3"],
					grant.0.iter(),
					|mut b, entry| {
						b.push_bind(blake3.0.as_slice())
							.push_bind(entry.pkg().hash().0.as_slice());
					}
				)
				.await?;
			}
			Err(e)
				if e.as_database_error()
					.is_some_and(sqlx::error::DatabaseError::is_unique_violation) => {}
			Err(e) => return Err(e.into()),
		}

		Ok(blake3)
	}
}

trait AsExecutor {
	fn as_executor(&mut self) -> impl MySqlExecutor<'_>;
}

impl AsExecutor for &MySqlPool {
	fn as_executor(&mut self) -> impl MySqlExecutor<'_> {
		&**self
	}
}

impl AsExecutor for &mut sqlx::MySqlTransaction<'_> {
	fn as_executor(&mut self) -> impl MySqlExecutor<'_> {
		&mut ***self
	}
}
