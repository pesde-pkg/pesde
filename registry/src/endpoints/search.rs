use std::collections::HashMap;

use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use tantivy::{collector::Count, query::AllQuery, schema::Value, DateTime, Order};

use pesde::{
    names::PackageName,
    source::{git_index::GitBasedSource, pesde::IndexFile},
};

use crate::{error::Error, package::PackageResponse, AppState};

#[derive(Deserialize)]
pub struct Request {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    offset: Option<usize>,
}

pub async fn search_packages(
    app_state: web::Data<AppState>,
    request: web::Query<Request>,
) -> Result<impl Responder, Error> {
    let searcher = app_state.search_reader.searcher();
    let schema = searcher.schema();

    let id = schema.get_field("id").unwrap();

    let scope = schema.get_field("scope").unwrap();
    let name = schema.get_field("name").unwrap();
    let description = schema.get_field("description").unwrap();

    let query = request.query.as_deref().unwrap_or_default().trim();

    let query = if query.is_empty() {
        Box::new(AllQuery)
    } else {
        let mut query_parser = tantivy::query::QueryParser::for_index(
            searcher.index(),
            vec![scope, name, description],
        );
        query_parser.set_field_boost(scope, 2.0);
        query_parser.set_field_boost(name, 3.5);

        query_parser.parse_query(query)?
    };

    let (count, top_docs) = searcher
        .search(
            &query,
            &(
                Count,
                tantivy::collector::TopDocs::with_limit(50)
                    .and_offset(request.offset.unwrap_or_default())
                    .order_by_fast_field::<DateTime>("published_at", Order::Desc),
            ),
        )
        .unwrap();

    let source = app_state.source.lock().await;

    let top_docs = top_docs
        .into_iter()
        .map(|(_, doc_address)| {
            let doc = searcher.doc::<HashMap<_, _>>(doc_address).unwrap();

            let id = doc
                .get(&id)
                .unwrap()
                .as_str()
                .unwrap()
                .parse::<PackageName>()
                .unwrap();
            let (scope, name) = id.as_str();

            let versions: IndexFile = toml::de::from_str(
                &source
                    .read_file([scope, name], &app_state.project, None)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();

            let (latest_version, entry) = versions
                .iter()
                .max_by_key(|(v_id, _)| v_id.version())
                .unwrap();

            PackageResponse {
                name: id.to_string(),
                version: latest_version.version().to_string(),
                targets: versions
                    .iter()
                    .filter(|(v_id, _)| v_id.version() == latest_version.version())
                    .map(|(_, entry)| (&entry.target).into())
                    .collect(),
                description: entry.description.clone().unwrap_or_default(),
                published_at: versions
                    .values()
                    .max_by_key(|entry| entry.published_at)
                    .unwrap()
                    .published_at,
                license: entry.license.clone().unwrap_or_default(),
                authors: entry.authors.clone(),
                repository: entry.repository.clone().map(|url| url.to_string()),
            }
        })
        .collect::<Vec<_>>();

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "data": top_docs,
        "count": count,
    })))
}
