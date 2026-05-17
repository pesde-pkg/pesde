use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::backend::EntrySeq;

#[get("/v2/log/head")]
pub async fn http(app_state: web::Data<AppState>) -> ControllerResult {
	let head = handler(&app_state.database).await?;
	Ok(HttpResponse::Ok().json(HeadResponse { head }))
}

async fn handler(db: &Database) -> AppResult<EntrySeq> {
	Ok(query(db).await?)
}

async fn query(db: &Database) -> anyhow::Result<EntrySeq> {
	todo!()
}

#[derive(serde::Serialize)]
struct HeadResponse {
	head: EntrySeq,
}
