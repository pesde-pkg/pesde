use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::*;

use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;

#[get("/v2/log/entry/{pos}")]
pub async fn http(app_state: web::Data<AppState>, path: web::Path<u64>) -> ControllerResult {
	let pos = path.into_inner();
	let Some(entry) = handler(&app_state.database, pos).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};
	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(db: &Database, pos: u64) -> AppResult<Option<Entry>> {
	Ok(db.get_entry(pos).await?)
}
