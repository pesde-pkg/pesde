use std::num::NonZero;

use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;
use serde::Deserialize;

use crate::AppState;
use crate::api::log::Error;
use crate::shared::auth::ReadGuard;

#[derive(Debug, Deserialize)]
struct ConsistencyQuery {
	size_from: Option<NonZero<u64>>,
}

#[get("/log/head")]
pub(super) async fn http_v2(
	_access_guard: ReadGuard,
	app_state: web::Data<AppState>,
	query: web::Query<ConsistencyQuery>,
) -> Result<impl Responder, Error> {
	let Some(head) = handler(app_state.db.as_ref(), query.size_from).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(HttpResponse::Ok().json(head))
}

async fn handler(
	db: &dyn Backend,
	size_from: Option<NonZero<u64>>,
) -> Result<Option<LogHeadResponse>, Error> {
	let mmr = db.read_mmr().await?;

	if mmr.mmr_size() == 0 {
		return Ok(None);
	}

	let proof_paths = match size_from {
		Some(size_from) => mmr
			.gen_consistency_proof(size_from.get())
			.await?
			.proof_paths()
			.to_vec(),
		None => Vec::new(),
	};

	Ok(Some(LogHeadResponse {
		accumulator: MmrAccumulator {
			algorithm: CURRENT_HASH_ALGORITHM,
			peaks: mmr.get_accumulator().await?.into(),
		},
		mmr_size: mmr.mmr_size(),
		proof_paths,
	}))
}
