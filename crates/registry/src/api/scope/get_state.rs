use std::num::NonZero;

use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;
use serde::Deserialize;

use crate::AppState;
use crate::api::scope::error::Error;

#[derive(Debug, Deserialize)]
struct StateQuery {
	at_size: NonZero<u64>,
}

#[get("/scope/{scope_id}/state")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	path: web::Path<ScopeId>,
	query: web::Query<StateQuery>,
) -> Result<impl Responder, Error> {
	let head = handler(
		app_state.db.as_ref(),
		&path.into_inner(),
		query.into_inner(),
	)
	.await?;

	Ok(HttpResponse::Ok().json(head))
}

async fn handler(
	db: &dyn Backend,
	scope_id: &ScopeId,
	query: StateQuery,
) -> Result<ScopeStateResponse, Error> {
	db.scope_state(scope_id, query.at_size)
		.await
		.map_err(Into::into)
		.and_then(|s| s.ok_or(Error::ScopeNotFound))
}
