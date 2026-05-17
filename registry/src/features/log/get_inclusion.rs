use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::backend::EntrySeq;

#[derive(serde::Deserialize)]
struct InclusionQuery {
	seq: EntrySeq,
}

#[get("/v2/log/inclusion")]
pub async fn http(
	app_state: web::Data<AppState>,
	params: web::Query<InclusionQuery>,
) -> ControllerResult {
	let result = handler(&app_state.database, params.seq).await?;
	Ok(HttpResponse::Ok().json(result))
}

async fn handler(db: &Database, seq: EntrySeq) -> AppResult<InclusionResponse> {
	Ok(query(db, seq).await?)
}

async fn query(db: &Database, _seq: EntrySeq) -> anyhow::Result<InclusionResponse> {
	todo!()
}

#[derive(serde::Serialize)]
struct InclusionResponse {
	proof: Vec<String>,
}
