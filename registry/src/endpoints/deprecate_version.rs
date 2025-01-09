use crate::{
	auth::UserId,
	error::{ErrorResponse, RegistryError},
	git::push_changes,
	package::{read_package, read_scope_info},
	search::search_version_changed,
	AppState,
};
use actix_web::{http::Method, web, HttpRequest, HttpResponse};
use pesde::names::PackageName;
use std::collections::HashMap;

pub async fn deprecate_package_version(
	request: HttpRequest,
	app_state: web::Data<AppState>,
	path: web::Path<PackageName>,
	bytes: web::Bytes,
	user_id: web::ReqData<UserId>,
) -> Result<HttpResponse, RegistryError> {
	let deprecated = request.method() != Method::DELETE;
	let reason = if deprecated {
		match String::from_utf8(bytes.to_vec()).map(|s| s.trim().to_string()) {
			Ok(reason) if !reason.is_empty() => reason,
			Err(e) => {
				return Ok(HttpResponse::BadRequest().json(ErrorResponse {
					error: format!("invalid utf-8: {e}"),
				}))
			}
			_ => {
				return Ok(HttpResponse::BadRequest().json(ErrorResponse {
					error: "deprecating must have a non-empty reason".to_string(),
				}))
			}
		}
	} else {
		String::new()
	};
	let name = path.into_inner();
	let source = app_state.source.lock().await;

	let Some(scope_info) = read_scope_info(&app_state, name.scope(), &source).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	if !scope_info.owners.contains(&user_id.0) {
		return Ok(HttpResponse::Forbidden().finish());
	}

	let Some(mut file) = read_package(&app_state, &name, &source).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	if file.meta.deprecated == reason {
		return Ok(HttpResponse::Conflict().finish());
	}

	file.meta.deprecated = reason;

	let file_string = toml::to_string(&file)?;

	push_changes(
		&app_state,
		&source,
		name.scope().to_string(),
		HashMap::from([(name.name().to_string(), file_string.into_bytes())]),
		format!("{}deprecate {name}", if deprecated { "" } else { "un" },),
	)
	.await?;

	search_version_changed(&app_state, &name, &file);

	Ok(HttpResponse::Ok().body(format!(
		"{}deprecated {name}",
		if deprecated { "" } else { "un" },
	)))
}
