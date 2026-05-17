use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::names::PackageName;
use semver::Version;

#[get("/v2/package/{scope}/{name}/{version}")]
pub async fn http(
	app_state: web::Data<AppState>,
	path: web::Path<(pesde::names::Scope, pesde::names::Name, semver::Version)>,
) -> ControllerResult {
	let (scope, name, version) = path.into_inner();
	let package_name = PackageName::new(scope, name);
	handler(&app_state.database, &package_name, &version).await?;
	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &Database, name: &PackageName, version: &Version) -> AppResult<()> {
	query(db, name, version).await?;
	Ok(())
}

async fn query(
	db: &Database,
	_name: &PackageName,
	_version: &semver::Version,
) -> anyhow::Result<()> {
	todo!()
}
