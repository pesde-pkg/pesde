use crate::AppState;
use crate::shared::db::Database;
use crate::shared::db::DatabaseTransaction;
use crate::shared::db::EntryKind;
use crate::shared::db::Mmr as _;
use crate::util::AppError;
use crate::util::AppResult;
use crate::util::HttpResult;
use crate::util::NonUnique;
use actix_web::HttpResponse;
use actix_web::post;
use actix_web::web;
use pesde::signature::Signature;
use pesde::source::pesde::registry::*;
use sqlx::error::DatabaseError;

#[post("/identity")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	body: web::Json<RegisterIdentityEntry>,
) -> HttpResult {
	let body = body.into_inner();
	handler(&app_state.database, body).await?;
	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &Database, body: RegisterIdentityEntry) -> AppResult<()> {
	let Some((sig, body)) = body.into_verified(|body| &body.public_key) else {
		return Err(AppError::InvalidSignature);
	};

	let mut mmr = db.write_mmr().await?;
	let pos = mmr.push(&canonical_bytes(&body)).await?;

	if let Some(non_unique) = query(mmr.commit_and_update().await?, pos, sig, body).await? {
		return Err(non_unique.into());
	}

	Ok(())
}

async fn query(
	tx: DatabaseTransaction<'_>,
	pos: u64,
	sig: Signature,
	body: RegisterIdentityBody,
) -> anyhow::Result<Option<NonUnique>> {
	match tx {
		DatabaseTransaction::MySql(tx) => {
			let mut tx = tx.into_inner();

			sqlx::query!(
				r#"
                INSERT INTO LogEntry (pos, kind) VALUES (?, ?)
                "#,
				pos,
				EntryKind::RegisterIdentity
			)
			.execute(&mut *tx)
			.await?;

			let key_id = sqlx::query!(
				r#"
                INSERT INTO UsedPublicKey (algorithm, public_key) VALUES (?, ?)
                "#,
				body.public_key.kind().to_string(),
				body.public_key.data()
			)
			.execute(&mut *tx)
			.await;
			let key_id = match key_id {
				Ok(result) => result.last_insert_id(),
				Err(e)
					if e.as_database_error()
						.is_some_and(DatabaseError::is_unique_violation) =>
				{
					return Ok(Some(NonUnique::PublicKey));
				}
				Err(e) => return Err(e.into()),
			};

			let entry_result = sqlx::query!(
				r#"
                INSERT INTO RegisterIdentityLogEntry (pos, sig, identity_id, public_key_id) VALUES (?, ?, ?, ?)
                "#,
				pos,
				sig.to_string(),
                body.identity_id.0,
                key_id
			)
			.execute(&mut *tx).await;
			match entry_result {
				Ok(_) => {}
				Err(e)
					if e.as_database_error()
						.is_some_and(DatabaseError::is_unique_violation) =>
				{
					return Ok(Some(NonUnique::IdentityId));
				}
				Err(e) => return Err(e.into()),
			}

			tx.commit().await?;
		}
	}

	Ok(None)
}
