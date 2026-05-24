use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use merkleberg::mmriver::InclusionProof;
use pesde::source::pesde::registry::*;

#[get("/v2/log/inclusion/{seq}")]
pub async fn http(app_state: web::Data<AppState>, path: web::Path<EntrySeq>) -> ControllerResult {
	let seq = path.into_inner();
	let result = handler(&app_state.database, seq).await?;
	Ok(HttpResponse::Ok().json(result))
}

async fn handler(db: &Database, seq: EntrySeq) -> AppResult<InclusionProof<Sha256Merge>> {
	let mmr = db.read_mmr().await?;
	Ok(mmr.gen_inclusion_proof(seq.0).await?)
}
