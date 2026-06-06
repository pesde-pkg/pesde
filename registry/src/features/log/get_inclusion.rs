use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::HttpResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use merkleberg::mmriver::InclusionProof;
use pesde::source::pesde::registry::*;

#[get("/log/inclusion/{pos}")]
pub(super) async fn http_v2(app_state: web::Data<AppState>, path: web::Path<u64>) -> HttpResult {
	let pos = path.into_inner();
	let result = handler(&app_state.database, pos).await?;
	Ok(HttpResponse::Ok().json(result))
}

async fn handler(db: &Database, from: u64) -> AppResult<InclusionProof<CurrentMmrMerge>> {
	let mmr = db.read_mmr().await?;
	Ok(mmr.gen_inclusion_proof(from).await?)
}
