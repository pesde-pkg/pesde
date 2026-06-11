use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::post;
use actix_web::web;
use pesde::source::pesde::registry::*;

use crate::AppState;
use crate::features::scope::Error;
use crate::shared::auth::WriteGuard;
use crate::shared::db::Backend;
use crate::shared::db::ScopeControl;
use crate::shared::db::append_leaf;

#[post("/scope/manifest")]
pub(super) async fn http_v2(
	_access_guard: WriteGuard,
	app_state: web::Data<AppState>,
	body: web::Json<ManifestUpdateScopeEntry>,
) -> Result<impl Responder, Error> {
	handler(app_state.db.as_ref(), body.into_inner()).await?;
	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &dyn Backend, entry: ManifestUpdateScopeEntry) -> Result<(), Error> {
	let mut store = db.begin_write().await?;
	let author = db
		.author_key(&mut store, &entry.unsafe_body().author_identity)
		.await?
		.ok_or(Error::UnknownIdentity)?;
	let Some((sig, body)) = entry.into_verified_external(&author.key) else {
		return Err(Error::InvalidSignature);
	};

	let Some(access) = db
		.authorize_scope_write(&mut store, &body.scope, &author, ScopeControl::Owner)
		.await?
	else {
		return Err(Error::Unauthorized);
	};

	let (mut store, _) = append_leaf(store, access.pos, &body).await?;
	db.insert_manifest_update(&mut store, access.pos, &sig, &body)
		.await?;
	store.commit().await?;

	Ok(())
}
