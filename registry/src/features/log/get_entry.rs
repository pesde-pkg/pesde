use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::*;

use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::HttpResult;

#[get("/log/entry/{pos}")]
pub(super) async fn http_v2(app_state: web::Data<AppState>, path: web::Path<u64>) -> HttpResult {
	let pos = path.into_inner();
	let Some(entry) = handler(&app_state.database, pos).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};
	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(db: &Database, pos: u64) -> AppResult<Option<Entry>> {
	Ok(db.get_entry(pos).await?)
}
