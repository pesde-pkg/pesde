use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;

use crate::Db;
use crate::api::scope::error::Error;

#[get("/scope/{scope_id}/state")]
pub(super) async fn http_v2(db: Db, path: web::Path<ScopeId>) -> Result<impl Responder, Error> {
	let head = handler(db.as_ref(), &path.into_inner()).await?;

	Ok(HttpResponse::Ok().json(head))
}

async fn handler(db: &dyn Backend, scope_id: &ScopeId) -> Result<ScopeStateResponse, Error> {
	db.scope_state(scope_id)
		.await
		.map_err(Into::into)
		.and_then(|s| s.ok_or(Error::ScopeNotFound))
}
