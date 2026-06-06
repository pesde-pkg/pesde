use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::HttpResult;
use actix_web::HttpResponse;
use actix_web::post;
use actix_web::web;

#[post("/scope/{scope}/manifest")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	scope: web::Path<pesde::names::Scope>,
) -> HttpResult {
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
