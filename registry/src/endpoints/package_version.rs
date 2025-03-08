use actix_web::{http::header::ACCEPT, web, HttpRequest, HttpResponse};
use serde::Deserialize;

use crate::{
	endpoints::package_doc::find_package_doc,
	error::RegistryError,
	package::{read_package, PackageResponse},
	request_path::{resolve_version_and_target, AnyOrSpecificTarget, LatestOrSpecificVersion},
	storage::StorageImpl as _,
	AppState,
};
use pesde::names::PackageName;

#[derive(Debug, Deserialize)]
pub struct Query {
	doc: Option<String>,
}

pub async fn get_package_version_v0(
	request: HttpRequest,
	app_state: web::Data<AppState>,
	path: web::Path<(PackageName, LatestOrSpecificVersion, AnyOrSpecificTarget)>,
	request_query: web::Query<Query>,
) -> Result<HttpResponse, RegistryError> {
	let (name, version, target) = path.into_inner();

	let Some(file) = read_package(&app_state, &name, &*app_state.source.read().await).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	let Some(v_id) = resolve_version_and_target(&file, version, &target) else {
		return Ok(HttpResponse::NotFound().finish());
	};

	if let Some(doc_name) = request_query.doc.as_deref() {
		let Some(hash) = find_package_doc(&file, v_id, doc_name) else {
			return Ok(HttpResponse::NotFound().finish());
		};

		return app_state.storage.get_doc(hash).await;
	}

	let accept = request
		.headers()
		.get(ACCEPT)
		.and_then(|accept| accept.to_str().ok())
		.and_then(|accept| match accept.to_lowercase().as_str() {
			"text/plain" => Some(true),
			"application/octet-stream" => Some(false),
			_ => None,
		});

	if let Some(readme) = accept {
		return if readme {
			app_state.storage.get_readme(&name, v_id).await
		} else {
			app_state.storage.get_package(&name, v_id).await
		};
	}

	Ok(HttpResponse::Ok().json(PackageResponse::new(&name, v_id, &file)))
}

pub async fn get_package_version(
	app_state: web::Data<AppState>,
	path: web::Path<(PackageName, LatestOrSpecificVersion, AnyOrSpecificTarget)>,
) -> Result<HttpResponse, RegistryError> {
	let (name, version, target) = path.into_inner();

	let Some(file) = read_package(&app_state, &name, &*app_state.source.read().await).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	let Some(v_id) = resolve_version_and_target(&file, version, &target) else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(HttpResponse::Ok().json(PackageResponse::new(&name, v_id, &file)))
}
