use std::num::NonZeroU64;

use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::HttpResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::*;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ConsistencyQuery {
	size_from: Option<NonZeroU64>,
}

#[get("/log/head")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	query: web::Query<ConsistencyQuery>,
) -> HttpResult {
	let Some(head) = handler(&app_state.database, query.size_from).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(HttpResponse::Ok().json(head))
}

async fn handler(
	db: &Database,
	size_from: Option<NonZeroU64>,
) -> AppResult<Option<LogHeadResponse>> {
	let mmr = db.read_mmr().await?;

	Ok(Some(LogHeadResponse {
		accumulator: MmrAccumulator {
			algorithm: CURRENT_HASH_ALGORITHM,
			peaks: mmr.get_accumulator().await?.into(),
		},
		state: match size_from {
			Some(size_from) => LogHeadResponseState::WithPreviousState {
				proof: mmr.gen_consistency_proof(size_from.get()).await?,
			},
			None => LogHeadResponseState::OnlyNewState {
				mmr_size_to: mmr.mmr_size(),
			},
		},
	}))
}
