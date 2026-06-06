use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::HttpResult;
use actix_web::HttpResponse;
use actix_web::post;
use actix_web::web;
use pesde::names::Name;
use pesde::names::PackageName;
use pesde::names::Scope;
use semver::Version;

#[post("/package/{scope}/{name}/{version}/yank")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	path: web::Path<(Scope, Name, semver::Version)>,
) -> HttpResult {
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
