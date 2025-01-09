use actix_web::{http::header::ACCEPT, web, HttpRequest, HttpResponse};
use serde::Deserialize;

use crate::{
	error::RegistryError,
	package::{read_package, PackageResponse},
	request_path::{AnyOrSpecificTarget, LatestOrSpecificVersion},
	storage::StorageImpl,
	AppState,
};
use pesde::{names::PackageName, source::pesde::DocEntryKind};

#[derive(Debug, Deserialize)]
pub struct Query {
	doc: Option<String>,
}

pub async fn get_package_version(
	request: HttpRequest,
	app_state: web::Data<AppState>,
	path: web::Path<(PackageName, LatestOrSpecificVersion, AnyOrSpecificTarget)>,
	request_query: web::Query<Query>,
) -> Result<HttpResponse, RegistryError> {
	let (name, version, target) = path.into_inner();

	let Some(file) = read_package(&app_state, &name, &*app_state.source.lock().await).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	let Some((v_id, entry)) = ({
		let version = match version {
			LatestOrSpecificVersion::Latest => match file.entries.keys().map(|k| k.version()).max()
			{
				Some(latest) => latest.clone(),
				None => return Ok(HttpResponse::NotFound().finish()),
			},
			LatestOrSpecificVersion::Specific(version) => version,
		};

		let mut versions = file
			.entries
			.iter()
			.filter(|(v_id, _)| *v_id.version() == version);

		match target {
			AnyOrSpecificTarget::Any => versions.min_by_key(|(v_id, _)| *v_id.target()),
			AnyOrSpecificTarget::Specific(kind) => {
				versions.find(|(_, entry)| entry.target.kind() == kind)
			}
		}
	}) else {
		return Ok(HttpResponse::NotFound().finish());
	};

	if let Some(doc_name) = request_query.doc.as_deref() {
		let hash = 'finder: {
			let mut queue = entry.docs.iter().map(|doc| &doc.kind).collect::<Vec<_>>();
			while let Some(doc) = queue.pop() {
				match doc {
					DocEntryKind::Page { name, hash } if name == doc_name => {
						break 'finder hash.clone()
					}
					DocEntryKind::Category { items, .. } => {
						queue.extend(items.iter().map(|item| &item.kind))
					}
					_ => continue,
				};
			}

			return Ok(HttpResponse::NotFound().finish());
		};

		return app_state.storage.get_doc(&hash).await;
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
