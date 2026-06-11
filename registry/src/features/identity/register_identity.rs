use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::post;
use actix_web::web;
use pesde::source::pesde::registry::*;

use crate::AppState;
use crate::features::identity::Error;
use crate::shared::auth::WriteGuard;
use crate::shared::db::Backend;
use crate::shared::db::append_leaf;

#[post("/identity")]
pub(super) async fn http_v2(
	_access_guard: WriteGuard,
	app_state: web::Data<AppState>,
	body: web::Json<RegisterIdentityEntry>,
) -> Result<impl Responder, Error> {
	handler(app_state.db.as_ref(), body.into_inner()).await?;
	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &dyn Backend, entry: RegisterIdentityEntry) -> Result<(), Error> {
	let Some((sig, body)) = entry.into_verified(|body| &body.public_key) else {
		return Err(Error::InvalidSignature);
	};

	let mut store = db.begin_write().await?;
	let pos = db.lock_tree(&mut store).await?;
	let (mut store, _) = append_leaf(store, pos, &body).await?;
	db.insert_register(&mut store, pos, &sig, &body).await?;
	store.commit().await?;

	Ok(())
}
