use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::post;
use actix_web::web;

#[post("/v2/identity/rotate")]
pub async fn http(app_state: web::Data<AppState>) -> ControllerResult {
	handler(&app_state.database).await?;
	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &Database) -> AppResult<()> {
	query(db).await?;
	Ok(())
}

async fn query(db: &Database) -> anyhow::Result<()> {
	todo!()
}
