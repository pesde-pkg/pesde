use crate::{
	error::RegistryError,
	package::{read_package, PackageResponse},
	search::find_max_searchable,
	AppState,
};
use actix_web::{web, HttpResponse};
use pesde::names::PackageName;
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use tantivy::{collector::Count, query::AllQuery, schema::Value as _, DateTime, Order};
use tokio::task::JoinSet;

#[derive(Deserialize)]
pub struct Request {
	#[serde(default)]
	query: String,
	#[serde(default)]
	offset: usize,
}

pub async fn search_packages(
	app_state: web::Data<AppState>,
	request_query: web::Query<Request>,
) -> Result<HttpResponse, RegistryError> {
	let searcher = app_state.search_reader.searcher();
	let schema = searcher.schema();

	let id_field = schema.get_field("id").unwrap();

	let query = request_query.query.trim();

	let (count, top_docs) = if query.is_empty() {
		let (count, top_docs) = searcher
			.search(
				&AllQuery,
				&(
					Count,
					tantivy::collector::TopDocs::with_limit(50)
						.and_offset(request_query.offset)
						.order_by_fast_field::<DateTime>("published_at", Order::Desc),
				),
			)
			.unwrap();

		let top_docs = top_docs
			.into_iter()
			.map(|(_, addr)| addr)
			.collect::<Vec<_>>();

		(count, top_docs)
	} else {
		let (count, top_docs) = searcher
			.search(
				&app_state.query_parser.parse_query(query)?,
				&(
					Count,
					tantivy::collector::TopDocs::with_limit(50).and_offset(request_query.offset),
				),
			)
			.unwrap();

		let mut top_docs = top_docs
			.into_iter()
			.map(|(score, addr)| {
				let segment_reader = searcher.segment_reader(addr.segment_ord);
				let fast_field_reader = segment_reader.fast_fields().date("published_at").unwrap();
				let published_at = fast_field_reader.first(addr.doc_id).unwrap();

				(score, published_at, addr)
			})
			.collect::<Vec<_>>();

		top_docs.sort_by(|a, b| {
			b.0.partial_cmp(&a.0)
				.unwrap_or(std::cmp::Ordering::Equal)
				.then_with(|| b.1.cmp(&a.1))
		});

		let top_docs = top_docs
			.into_iter()
			.map(|(_, _, addr)| addr)
			.collect::<Vec<_>>();

		(count, top_docs)
	};

	let source = Arc::new(app_state.source.clone().read_owned().await);

	let mut results = top_docs
		.iter()
		.map(|_| None::<PackageResponse>)
		.collect::<Vec<_>>();

	let mut tasks = top_docs
		.into_iter()
		.enumerate()
		.map(|(i, doc_address)| {
			let doc = searcher.doc::<HashMap<_, _>>(doc_address).unwrap();
			let id = (&doc[&id_field])
				.as_str()
				.unwrap()
				.parse::<PackageName>()
				.unwrap();

			let app_state = app_state.clone();
			let source = source.clone();

			async move {
				let file = read_package(&app_state, &id, &source).await?.unwrap();

				let (version_id, _) = find_max_searchable(&file).unwrap();

				Ok::<_, RegistryError>((i, PackageResponse::new(&id, version_id, &file)))
			}
		})
		.collect::<JoinSet<_>>();

	while let Some(res) = tasks.join_next().await {
		let (i, res) = res.unwrap()?;
		results[i] = Some(res);
	}

	Ok(HttpResponse::Ok().json(serde_json::json!({
		"data": results,
		"count": count,
	})))
}
