use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde_core::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::features::scope::ScopeReadRepository;
use pesde_registry_core::features::scope::Scoped;

use crate::Db;
use crate::api::scope::error::Error;
use crate::shared::log::LogHeadQuery;
use crate::shared::log::log_head;

#[get("/scope/{scope_id}/log/head")]
pub(super) async fn http_v2(
	db: Db,
	path: web::Path<ScopeId>,
	query: web::Query<LogHeadQuery>,
) -> Result<impl Responder, Error> {
	let head = handler(db.as_ref(), &path.into_inner(), query.into_inner()).await?;

	Ok(HttpResponse::Ok().json(head))
}

async fn handler(
	db: &dyn Backend,
	scope_id: &ScopeId,
	query: LogHeadQuery,
) -> Result<LogHeadResponse, Error> {
	let current_size = db
		.scope_log_size(scope_id)
		.await?
		.ok_or(Error::ScopeNotFound)?;

	log_head(
		current_size,
		Scoped(scope_id, db as &dyn ScopeReadRepository),
		query,
	)
	.await
}
