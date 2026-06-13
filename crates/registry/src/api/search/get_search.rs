use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use serde::Deserialize;

use crate::AppState;
use crate::api::search::error::Error;
use crate::shared::auth::ReadGuard;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

#[derive(Debug, Deserialize)]
struct SearchQuery {
	#[serde(default, alias = "q")]
	query: String,
	#[serde(default)]
	limit: Option<usize>,
	#[serde(default)]
	offset: usize,
}

#[get("/search")]
pub(super) async fn http_v2(
	_access_guard: ReadGuard,
	app_state: web::Data<AppState>,
	query: web::Query<SearchQuery>,
) -> Result<impl Responder, Error> {
	let query = query.into_inner();
	let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);

	let result = app_state
		.search
		.query(&*app_state.db, query.query, limit, query.offset)
		.await?;

	Ok(HttpResponse::Ok().json(result))
}
