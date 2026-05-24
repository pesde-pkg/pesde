use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::*;

use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;

#[get("/v2/log/entry/{seq}")]
pub async fn http(app_state: web::Data<AppState>, seq: web::Path<EntrySeq>) -> ControllerResult {
	let Some(entry) = handler(&app_state.database, seq.into_inner()).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};
	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(db: &Database, seq: EntrySeq) -> AppResult<Option<Entry>> {
	match db {
		Database::MySql(pool) => crate::shared::db::mysql::get_entry(pool, seq)
			.await
			.map_err(Into::into),
	}
}
