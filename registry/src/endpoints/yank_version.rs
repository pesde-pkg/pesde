use crate::{
	auth::UserId,
	error::RegistryError,
	git::push_changes,
	package::{read_package, read_scope_info},
	request_path::AllOrSpecificTarget,
	search::search_version_changed,
	AppState,
};
use actix_web::{http::Method, web, HttpRequest, HttpResponse};
use pesde::names::PackageName;
use semver::Version;
use std::collections::HashMap;

pub async fn yank_package_version(
	request: HttpRequest,
	app_state: web::Data<AppState>,
	path: web::Path<(PackageName, Version, AllOrSpecificTarget)>,
	user_id: web::ReqData<UserId>,
) -> Result<HttpResponse, RegistryError> {
	let yanked = request.method() != Method::DELETE;
	let (name, version, target) = path.into_inner();
	let source = app_state.source.write().await;

	let Some(scope_info) = read_scope_info(&app_state, name.scope(), &source).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	if !scope_info.owners.contains(&user_id.0) {
		return Ok(HttpResponse::Forbidden().finish());
	}

	let Some(mut file) = read_package(&app_state, &name, &source).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	let mut targets = vec![];

	for (v_id, entry) in &mut file.entries {
		if *v_id.version() != version {
			continue;
		}

		match target {
			AllOrSpecificTarget::Specific(kind) if entry.target.kind() != kind => continue,
			_ => {}
		}

		if entry.yanked == yanked {
			continue;
		}

		targets.push(entry.target.kind().to_string());
		entry.yanked = yanked;
	}

	if targets.is_empty() {
		return Ok(HttpResponse::Conflict().finish());
	}

	let file_string = toml::to_string(&file)?;

	push_changes(
		&app_state,
		&source,
		name.scope().to_string(),
		HashMap::from([(name.name().to_string(), file_string.into_bytes())]),
		format!(
			"{}yank {name}@{version} {}",
			if yanked { "" } else { "un" },
			targets.join(", "),
		),
	)
	.await?;

	search_version_changed(&app_state, &name, &file);

	Ok(HttpResponse::Ok().body(format!(
		"{}yanked {name}@{version} {}",
		if yanked { "" } else { "un" },
		targets.join(", "),
	)))
}
