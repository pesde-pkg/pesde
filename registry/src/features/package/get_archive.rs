use crate::AppState;
use crate::shared::blob::BlobStorage;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::names::Name;
use pesde::names::PackageName;
use pesde::names::Scope;

#[get("/v2/package/{scope}/{name}/{version}/archive")]
pub async fn http(
	app_state: web::Data<AppState>,
	path: web::Path<(Scope, Name, semver::Version)>,
) -> ControllerResult {
	let (scope, name, version) = path.into_inner();
	let package_name = PackageName::new(scope, name);

	let Some(response) = handler(&app_state.blob_storage, &package_name, &version).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(response.into())
}

async fn handler(
	blob: &BlobStorage,
	name: &PackageName,
	version: &semver::Version,
) -> AppResult<Option<crate::shared::blob::BlobResponse>> {
	blob.get_package_archive(name, version)
		.await
		.map_err(Into::into)
}
