use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::HttpResult;
use actix_web::HttpResponse;
use actix_web::post;
use actix_web::web;
use pesde::names::PackageName;

#[post("/package/{scope}/{name}/deprecate")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	path: web::Path<(pesde::names::Scope, pesde::names::Name)>,
) -> HttpResult {
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
