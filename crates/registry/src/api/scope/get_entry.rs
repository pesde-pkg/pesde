use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use futures::TryFutureExt as _;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::features::scope::Scoped;

use crate::Db;
use crate::api::scope::error::Error;
use crate::shared::log::LogEntryQuery;
use crate::shared::log::log_entry;

#[get("/scope/{scope_id}/log/entry/{pos}")]
pub(super) async fn http_v2(
	db: Db,
	path: web::Path<(ScopeId, u64)>,
	query: web::Query<LogEntryQuery>,
) -> Result<impl Responder, Error> {
	let (scope_id, pos) = path.into_inner();
	let entry = handler(db.as_ref(), scope_id, pos, query.into_inner()).await?;

	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(
	db: &dyn Backend,
	scope_id: ScopeId,
	pos: u64,
	query: LogEntryQuery,
) -> Result<LogEntryResponse<ScopeEntryPayload>, Error> {
	log_entry(
		pos,
		|pos| {
			db.scope_log_entry(scope_id.clone(), pos)
				.map_err(Into::into)
		},
		async || {
			let size = db
				.scope_log_size(&scope_id)
				.await?
				.ok_or(Error::ScopeNotFound)?;
			Ok((size, Scoped(&scope_id, db)))
		},
		query,
	)
	.await
	.and_then(|e| e.ok_or(Error::ScopeNotFound))
}
