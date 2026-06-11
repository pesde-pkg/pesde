use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::Entry;
use pesde::source::pesde::registry::EntryPayload;

use crate::AppState;
use crate::features::log::Error;
use crate::shared::auth::ReadGuard;
use crate::shared::db::Backend;

#[get("/log/entry/{pos}")]
pub(super) async fn http_v2(
	_access_guard: ReadGuard,
	app_state: web::Data<AppState>,
	path: web::Path<u64>,
) -> Result<impl Responder, Error> {
	let Some(entry) = handler(app_state.db.as_ref(), path.into_inner()).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(db: &dyn Backend, pos: u64) -> anyhow::Result<Option<Entry<EntryPayload>>> {
	db.entry(pos).await
}
