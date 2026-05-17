use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::names::Name;
use pesde::names::PackageName;
use pesde::names::Scope;

#[get("/v2/package/{scope}/{name}")]
pub async fn http(
	app_state: web::Data<AppState>,
	path: web::Path<(Scope, Name)>,
) -> ControllerResult {
	let (scope, name) = path.into_inner();
	let package_name = PackageName::new(scope, name);
	handler(&app_state.database, &package_name).await?;
	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &Database, name: &PackageName) -> AppResult<()> {
	query(db, name).await?;
	Ok(())
}

async fn query(db: &Database, _name: &PackageName) -> anyhow::Result<()> {
	todo!()
}
