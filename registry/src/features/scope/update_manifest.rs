use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::post;
use actix_web::web;

#[post("/v2/scope/{scope}/manifest")]
pub async fn http(
	app_state: web::Data<AppState>,
	scope: web::Path<pesde::names::Scope>,
) -> ControllerResult {
	handler(&app_state.database, &scope).await?;
	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &Database, scope: &pesde::names::Scope) -> AppResult<()> {
	query(db, scope).await?;
	Ok(())
}

async fn query(db: &Database, _scope: &pesde::names::Scope) -> anyhow::Result<()> {
	todo!()
}
