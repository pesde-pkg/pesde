use std::num::NonZero;

use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;

use crate::AppState;
use crate::api::log::error::Error;
use crate::shared::log::LogHeadQuery;
use crate::shared::log::log_head;

#[get("/log/head")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	query: web::Query<LogHeadQuery>,
) -> Result<impl Responder, Error> {
	let Some(head) = handler(app_state.db.as_ref(), query.into_inner()).await? else {
		return Ok(HttpResponse::NoContent().finish());
	};

	Ok(HttpResponse::Ok().json(head))
}

async fn handler(db: &dyn Backend, query: LogHeadQuery) -> Result<Option<LogHeadResponse>, Error> {
	let current_size = db.global_log_size().await?;

	if let Some(current_size) = NonZero::new(current_size) {
		log_head(current_size, &*db.global_mmr_read_store(), query)
			.await
			.map(Some)
	} else {
		Ok(None)
	}
}
