use std::num::NonZero;

use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use futures::TryFutureExt as _;
use pesde::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::features::global::Global;

use crate::Db;
use crate::api::log::error::Error;
use crate::shared::log::LogHeadQuery;
use crate::shared::log::log_head;

#[get("/log/head")]
pub(super) async fn http_v2(
	db: Db,
	query: web::Query<LogHeadQuery>,
) -> Result<impl Responder, Error> {
	let Some(head) = handler(db.as_ref(), query.into_inner()).await? else {
		return Ok(HttpResponse::NoContent().finish());
	};

	Ok(HttpResponse::Ok().json(head))
}

async fn handler(
	db: &dyn Backend,
	query: LogHeadQuery,
) -> Result<Option<LogHeadResponse<GlobalEntryPayload>>, Error> {
	let current_size = db.global_log_size().await?;

	if let Some(current_size) = NonZero::new(current_size) {
		log_head(
			current_size,
			Global(db),
			|index| db.global_log_entry(index).map_err(Into::into),
			query,
		)
		.await
		.map(Some)
	} else {
		Ok(None)
	}
}
