use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::post;
use actix_web::web;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;

use crate::AppState;
use crate::api::identity::error::Error;
use crate::shared::auth::WriteGuard;
use crate::shared::db::append_leaf;

#[post("/identity/rotate")]
pub(super) async fn http_v2(
	_access_guard: WriteGuard,
	app_state: web::Data<AppState>,
	body: web::Json<IdentityRotationEntry>,
) -> Result<impl Responder, Error> {
	handler(app_state.db.as_ref(), body.into_inner()).await?;
	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &dyn Backend, entry: IdentityRotationEntry) -> Result<(), Error> {
	let mut store = db.begin_write().await?;
	let author = db
		.author_key(&mut store, &entry.unsafe_body().identity_id)
		.await?
		.ok_or(Error::UnknownIdentity)?;
	let Some((old_sig, new_sig, body)) = entry.into_verified(&author.key) else {
		return Err(Error::InvalidSignature);
	};

	let Some(pos) = db.lock_tree_as(&mut store, &author).await? else {
		return Err(Error::InvalidSignature);
	};
	let (mut store, _) = append_leaf(store, pos, &body).await?;
	db.insert_rotation(&mut store, pos, &old_sig, &new_sig, &body)
		.await?;
	store.commit().await?;

	Ok(())
}
