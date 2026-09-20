use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use merkleberg::MMRIVER;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;

use crate::AppState;
use crate::api::scope::error::Error;
use crate::shared::log::LogEntryQuery;
use crate::shared::log::log_entry;

#[get("/scope/{scope_id}/log/entry/{pos}")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	path: web::Path<(ScopeId, u64)>,
	query: web::Query<LogEntryQuery>,
) -> Result<impl Responder, Error> {
	let (scope_id, pos) = path.into_inner();
	let Some(entry) = handler(app_state.db.as_ref(), &scope_id, pos, query.into_inner()).await?
	else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(
	db: &dyn Backend,
	scope_id: &ScopeId,
	pos: u64,
	query: LogEntryQuery,
) -> Result<Option<LogEntryResponse<ScopeEntryPayload>>, Error> {
	log_entry(
		pos,
		|pos| db.scope_log_entry(scope_id, pos),
		async || {
			let size = db
				.scope_log_size(scope_id)
				.await?
				.ok_or(Error::ScopeNotFound)?;
			Ok(MMRIVER::new(size.get(), db.scope_mmr_read_store(scope_id)))
		},
		query,
	)
	.await
}
