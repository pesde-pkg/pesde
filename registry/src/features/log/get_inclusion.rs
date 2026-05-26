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
	let Some(result) = handler(&app_state.database, seq).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};
	Ok(HttpResponse::Ok().json(result))
}

async fn handler(db: &Database, from: EntrySeq) -> AppResult<Option<InclusionProof<Sha256Merge>>> {
	let mmr = db.read_mmr().await?;
	let Some(pos) = db.get_pos(from).await? else {
		return Ok(None);
	};
	Ok(Some(mmr.gen_inclusion_proof(pos).await?))
}
