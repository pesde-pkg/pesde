use std::num::NonZero;

use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::InclusionProofResponse;
use pesde_registry_core::db::Backend;
use serde::Deserialize;

use crate::AppState;
use crate::api::log::Error;
use crate::shared::auth::ReadGuard;

#[derive(Debug, Deserialize)]
struct InclusionQuery {
	mmr_size: NonZero<u64>,
}

#[get("/log/inclusion/{pos}")]
pub(super) async fn http_v2(
	_access_guard: ReadGuard,
	app_state: web::Data<AppState>,
	path: web::Path<u64>,
	query: web::Query<InclusionQuery>,
) -> Result<impl Responder, Error> {
	let response = handler(
		app_state.db.as_ref(),
		path.into_inner(),
		query.mmr_size.get(),
	)
	.await?;

	Ok(HttpResponse::Ok().json(response))
}

async fn handler(
	db: &dyn Backend,
	from: u64,
	mmr_size: u64,
) -> Result<InclusionProofResponse, Error> {
	let current = db.current_size().await?;
	if mmr_size > current {
		return Err(Error::SizeOutOfRange {
			requested: mmr_size,
			current,
		});
	}

	let mmr = db.read_mmr_at(mmr_size).await?;
	let proof = mmr.gen_inclusion_proof(from).await?;

	Ok(InclusionProofResponse {
		index: proof.index(),
		proof: proof.proof().to_vec(),
	})
}
