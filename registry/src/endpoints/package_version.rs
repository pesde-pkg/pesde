use actix_web::{http::header::ACCEPT, web, HttpRequest, HttpResponse, Responder};
use semver::Version;
use serde::{Deserialize, Deserializer};

use crate::{error::Error, package::PackageResponse, storage::StorageImpl, AppState};
use pesde::{
    manifest::target::TargetKind,
    names::PackageName,
    source::{
        git_index::{read_file, root_tree, GitBasedSource},
        pesde::{DocEntryKind, IndexFile},
    },
};

#[derive(Debug)]
pub enum VersionRequest {
    Latest,
    Specific(Version),
}

impl<'de> Deserialize<'de> for VersionRequest {
    fn deserialize<D>(deserializer: D) -> Result<VersionRequest, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s.eq_ignore_ascii_case("latest") {
            return Ok(VersionRequest::Latest);
        }

        s.parse()
            .map(VersionRequest::Specific)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug)]
pub enum TargetRequest {
    Any,
    Specific(TargetKind),
}

impl<'de> Deserialize<'de> for TargetRequest {
    fn deserialize<D>(deserializer: D) -> Result<TargetRequest, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s.eq_ignore_ascii_case("any") {
            return Ok(TargetRequest::Any);
        }

        s.parse()
            .map(TargetRequest::Specific)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Deserialize)]
pub struct Query {
    doc: Option<String>,
}

pub async fn get_package_version(
    request: HttpRequest,
    app_state: web::Data<AppState>,
    path: web::Path<(PackageName, VersionRequest, TargetRequest)>,
    query: web::Query<Query>,
) -> Result<impl Responder, Error> {
    let (name, version, target) = path.into_inner();

    let (scope, name_part) = name.as_str();

    let entries: IndexFile = {
        let source = app_state.source.lock().await;
        let repo = gix::open(source.path(&app_state.project))?;
        let tree = root_tree(&repo)?;

        match read_file(&tree, [scope, name_part])? {
            Some(versions) => toml::de::from_str(&versions)?,
            None => return Ok(HttpResponse::NotFound().finish()),
        }
    };

    let Some((v_id, entry, targets)) = ({
        let version = match version {
            VersionRequest::Latest => match entries.keys().map(|k| k.version()).max() {
                Some(latest) => latest.clone(),
                None => return Ok(HttpResponse::NotFound().finish()),
            },
            VersionRequest::Specific(version) => version,
        };

        let versions = entries
            .iter()
            .filter(|(v_id, _)| *v_id.version() == version);

        match target {
            TargetRequest::Any => versions.clone().min_by_key(|(v_id, _)| *v_id.target()),
            TargetRequest::Specific(kind) => versions
                .clone()
                .find(|(_, entry)| entry.target.kind() == kind),
        }
        .map(|(v_id, entry)| {
            (
                v_id,
                entry,
                versions.map(|(_, entry)| (&entry.target).into()).collect(),
            )
        })
    }) else {
        return Ok(HttpResponse::NotFound().finish());
    };

    if let Some(doc_name) = query.doc.as_deref() {
        let hash = 'finder: {
            let mut hash = entry.docs.iter().map(|doc| &doc.kind).collect::<Vec<_>>();
            while let Some(doc) = hash.pop() {
                match doc {
                    DocEntryKind::Page { name, hash } if name == doc_name => {
                        break 'finder hash.clone()
                    }
                    DocEntryKind::Category { items, .. } => {
                        hash.extend(items.iter().map(|item| &item.kind))
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

    let response = PackageResponse {
        name: name.to_string(),
        version: v_id.version().to_string(),
        targets,
        description: entry.description.clone().unwrap_or_default(),
        published_at: entry.published_at,
        license: entry.license.clone().unwrap_or_default(),
        authors: entry.authors.clone(),
        repository: entry.repository.clone().map(|url| url.to_string()),
    };

    let mut value = serde_json::to_value(response)?;
    value["docs"] = serde_json::to_value(entry.docs.clone())?;
    value["dependencies"] = serde_json::to_value(entry.dependencies.clone())?;

    Ok(HttpResponse::Ok().json(value))
}
