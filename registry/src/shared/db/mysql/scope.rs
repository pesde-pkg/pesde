use async_trait::async_trait;
use itertools::Either;
use pesde::names::Name;
use pesde::signature::Signature;
use pesde::source::pesde::registry::*;
use sqlx::error::DatabaseError;

use crate::features::scope::Repository;
use crate::shared::db::ManifestError;
use crate::shared::db::WriteStore;
use crate::shared::db::mysql::MySqlBackend;
use crate::shared::db::mysql::ScopeEntryKind;
use crate::shared::db::mysql::as_tx;
use crate::shared::db::mysql::insert_scope_envelope;

#[async_trait]
impl Repository for MySqlBackend {
	async fn insert_manifest_update(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &ScopeEntryBody<ScopeManifestUpdateBody>,
	) -> Result<(), ManifestError> {
		let conn = as_tx(tx);

		let scope_id = insert_scope_envelope(conn, pos, sig, body, ScopeEntryKind::ManifestUpdate)
			.await
			.map_err(ManifestError::Internal)?;

		let manifest = &body.payload.manifest;
		match sqlx::query!(
			"INSERT INTO ScopeManifest (pos, scope_id, owner) VALUES (?, ?, ?)",
			pos,
			scope_id,
			&manifest.owner,
		)
		.execute(&mut **conn)
		.await
		{
			Ok(_) => {}
			Err(e)
				if e.as_database_error()
					.is_some_and(DatabaseError::is_foreign_key_violation) =>
			{
				return Err(ManifestError::UnregisteredIdentity(manifest.owner));
			}
			Err(e) => return Err(anyhow::Error::from(e).into()),
		}

		for (identity_id, member) in &manifest.members {
			let packages = match member {
				ScopeMember::AllPackages => Either::Left(std::iter::once("")),
				ScopeMember::Packages(packages) => Either::Right(packages.iter().map(Name::as_str)),
			};

			for package in packages {
				match sqlx::query!(
					"INSERT INTO ScopeManifestMember (pos, identity_id, package) VALUES (?, ?, ?)",
					pos,
					identity_id,
					package,
				)
				.execute(&mut **conn)
				.await
				{
					Ok(_) => {}
					Err(e)
						if e.as_database_error()
							.is_some_and(DatabaseError::is_foreign_key_violation) =>
					{
						return Err(ManifestError::UnregisteredIdentity(*identity_id));
					}
					Err(e) => return Err(anyhow::Error::from(e).into()),
				}
			}
		}

		Ok(())
	}
}
