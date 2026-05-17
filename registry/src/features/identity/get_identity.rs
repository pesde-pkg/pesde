use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::backend::IdentityId;

#[get("/v2/identity/{identity_id}")]
pub async fn http(
	app_state: web::Data<AppState>,
	identity_id: web::Path<IdentityId>,
) -> ControllerResult {
	handler(&app_state.database, &identity_id).await?;
	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &Database, identity_id: &IdentityId) -> AppResult<()> {
	query(db, identity_id).await?;
	Ok(())
}

async fn query(db: &Database, _identity_id: &IdentityId) -> anyhow::Result<()> {
	todo!()
}
