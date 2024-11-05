use std::collections::{BTreeMap, BTreeSet};

use actix_web::{web, HttpResponse, Responder};

use pesde::{
    names::PackageName,
    source::{git_index::GitBasedSource, pesde::IndexFile},
};

use crate::{error::Error, package::PackageResponse, AppState};

pub async fn get_package_versions(
    app_state: web::Data<AppState>,
    path: web::Path<PackageName>,
) -> Result<impl Responder, Error> {
    let name = path.into_inner();

    let (scope, name_part) = name.as_str();

    let source = app_state.source.lock().await;
    let versions: IndexFile =
        match source.read_file([scope, name_part], &app_state.project, None)? {
            Some(versions) => toml::de::from_str(&versions)?,
            None => return Ok(HttpResponse::NotFound().finish()),
        };

    let mut responses = BTreeMap::new();

    for (v_id, entry) in versions {
        let info = responses
            .entry(v_id.version().clone())
            .or_insert_with(|| PackageResponse {
                name: name.to_string(),
                version: v_id.version().to_string(),
                targets: BTreeSet::new(),
                description: entry.description.unwrap_or_default(),
                published_at: entry.published_at,
                license: entry.license.unwrap_or_default(),
                authors: entry.authors.clone(),
                repository: entry.repository.clone().map(|url| url.to_string()),
            });

        info.targets.insert(entry.target.into());
        info.published_at = info.published_at.max(entry.published_at);
    }

    Ok(HttpResponse::Ok().json(responses.into_values().collect::<Vec<_>>()))
}
