use anyhow::Context as _;
use async_trait::async_trait;
use jiff::Timestamp;
use pesde::signature::KeyKind;
use pesde::signature::PublicKey;
use pesde::signature::Signature;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::WriteStore;
use pesde_registry_core::features::identity::IdentityWriteError;
use pesde_registry_core::features::identity::Repository;
use sqlx::error::DatabaseError;

use crate::EntryKind;
use crate::MySqlBackend;
use crate::as_tx;
use crate::build_public_key;
use crate::insert_log_entry;

#[async_trait]
impl Repository for MySqlBackend {
	async fn identity_entry(
		&self,
		id: &IdentityId,
	) -> anyhow::Result<Option<Entry<IdentityEntry>>> {
		let Some(row) = sqlx::query!(
			r#"
            SELECT IdentityKeyEntry.pos, IdentityKeyEntry.sig, IdentityKeyEntry.authorising_sig, IdentityKeyEntry.algorithm AS `algorithm: KeyKind`, IdentityKeyEntry.public_key, LogEntry.kind AS `kind: EntryKind`, UNIX_TIMESTAMP(LogEntry.published_at) AS `published_at!`
            FROM IdentityKeyEntry
            INNER JOIN LogEntry ON LogEntry.pos=IdentityKeyEntry.pos
            WHERE IdentityKeyEntry.identity_id = ?
            ORDER BY IdentityKeyEntry.pos DESC
            LIMIT 1
            "#,
			id,
		)
		.fetch_optional(&self.pool)
		.await?
		else {
			return Ok(None);
		};

		let public_key = build_public_key(row.algorithm, row.public_key)?;
		let payload = if matches!(row.kind, EntryKind::IdentityRotation) {
			let old_sig = row
				.authorising_sig
				.context("rotation entry is missing its authorising signature")?;
			IdentityEntry::Rotation(IdentityRotationEntry::new(
				old_sig.parse()?,
				row.sig.parse()?,
				IdentityRotationBody {
					identity_id: *id,
					new_public_key: public_key,
				},
			))
		} else {
			IdentityEntry::Register(SignedEntry::new(
				row.sig.parse()?,
				RegisterIdentityBody {
					identity_id: *id,
					public_key,
				},
			))
		};

		Ok(Some(Entry {
			pos: row.pos,
			published_at: Timestamp::from_second(row.published_at)?,
			payload,
		}))
	}

	async fn insert_register(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &RegisterIdentityBody,
	) -> Result<(), IdentityWriteError> {
		let conn = as_tx(tx);

		insert_log_entry(conn, pos, EntryKind::RegisterIdentity)
			.await
			.map_err(anyhow::Error::from)?;

		match sqlx::query!(
			"INSERT INTO Identity (identity_id) VALUES (?)",
			&body.identity_id,
		)
		.execute(&mut **conn)
		.await
		{
			Ok(_) => {}
			Err(e)
				if e.as_database_error()
					.is_some_and(DatabaseError::is_unique_violation) =>
			{
				return Err(IdentityWriteError::NonUniqueIdentityId);
			}
			Err(e) => return Err(anyhow::Error::from(e).into()),
		}

		insert_key_entry(conn, pos, sig, None, &body.identity_id, &body.public_key).await
	}

	async fn insert_rotation(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		old_sig: &Signature,
		new_sig: &Signature,
		body: &IdentityRotationBody,
	) -> Result<(), IdentityWriteError> {
		let conn = as_tx(tx);

		insert_log_entry(conn, pos, EntryKind::IdentityRotation)
			.await
			.map_err(anyhow::Error::from)?;

		insert_key_entry(
			conn,
			pos,
			new_sig,
			Some(old_sig),
			&body.identity_id,
			&body.new_public_key,
		)
		.await
	}
}

async fn insert_key_entry(
	conn: &mut sqlx::MySqlTransaction<'_>,
	pos: u64,
	sig: &Signature,
	authorising_sig: Option<&Signature>,
	identity_id: &IdentityId,
	public_key: &PublicKey,
) -> Result<(), IdentityWriteError> {
	match sqlx::query!(
		"INSERT INTO IdentityKeyEntry (pos, sig, authorising_sig, identity_id, algorithm, public_key) VALUES (?, ?, ?, ?, ?, ?)",
		pos,
		sig.to_string(),
		authorising_sig.map(ToString::to_string),
		identity_id,
		public_key.kind(),
		public_key.data(),
	)
	.execute(&mut **conn)
	.await
	{
		Ok(_) => Ok(()),
		Err(e)
			if e.as_database_error()
				.is_some_and(DatabaseError::is_unique_violation) =>
		{
			Err(IdentityWriteError::NonUniquePublicKey)
		}
		Err(e) => Err(anyhow::Error::from(e).into()),
	}
}
