use crate::AppState;
use crate::shared::blob::BlobResponse;
use crate::shared::blob::BlobStorage;
use crate::util::AppResult;
use crate::util::HttpResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::names::Name;
use pesde::names::PackageName;
use pesde::names::Scope;
use semver::Version;

#[get("/package/{scope}/{name}/{version}/archive")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	path: web::Path<(Scope, Name, Version)>,
) -> HttpResult {
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
	version: &Version,
) -> AppResult<Option<BlobResponse>> {
	blob.get_package_archive(name, version)
		.await
		.map_err(Into::into)
}
