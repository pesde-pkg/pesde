use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::backend::EntrySeq;

#[derive(serde::Deserialize)]
struct ConsistencyQuery {
	from: EntrySeq,
	to: EntrySeq,
}

#[get("/v2/log/consistency")]
pub async fn http(
	app_state: web::Data<AppState>,
	params: web::Query<ConsistencyQuery>,
) -> ControllerResult {
	let result = handler(&app_state.database, params.from, params.to).await?;
	Ok(HttpResponse::Ok().json(result))
}

async fn handler(db: &Database, from: EntrySeq, to: EntrySeq) -> AppResult<ConsistencyResponse> {
	Ok(query(db, from, to).await?)
}

async fn query(
	db: &Database,
	_from: EntrySeq,
	_to: EntrySeq,
) -> anyhow::Result<ConsistencyResponse> {
	todo!()
}

#[derive(serde::Serialize)]
struct ConsistencyResponse {
	proof: Vec<String>,
}
