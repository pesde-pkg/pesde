use std::collections::HashMap;
use std::sync::Mutex;

use actix_web::web;
use anyhow::Context as _;
use futures::Stream;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::stream;
use itertools::Itertools as _;
use pesde::names::PackageName;
use pesde::source::pesde::registry::SearchResultItem;
use serde::Serialize;
use tantivy::DateTime;
use tantivy::Index;
use tantivy::IndexReader;
use tantivy::IndexWriter;
use tantivy::Order;
use tantivy::Term;
use tantivy::collector::Count;
use tantivy::collector::TopDocs;
use tantivy::doc;
use tantivy::query::AllQuery;
use tantivy::query::QueryParser;
use tantivy::schema::FAST;
use tantivy::schema::Field;
use tantivy::schema::IndexRecordOption;
use tantivy::schema::STORED;
use tantivy::schema::STRING;
use tantivy::schema::Schema;
use tantivy::schema::TextFieldIndexing;
use tantivy::schema::TextOptions;
use tantivy::schema::Value as _;
use tantivy::tokenizer::TextAnalyzer;

use crate::shared::db::Backend;

const WRITER_HEAP_BYTES: usize = 50 * 1024 * 1024;

pub struct Search {
	index: Index,
	reader: IndexReader,
	writer: Mutex<IndexWriter>,
	id: Field,
	scope: Field,
	name: Field,
	description: Field,
	published_at: Field,
}

#[derive(Debug, Serialize)]
pub struct QueryResult {
	count: usize,
	results: Vec<SearchResultItem>,
}

impl Search {
	pub async fn new(
		packages: impl Stream<Item = anyhow::Result<(PackageName, String)>>,
	) -> anyhow::Result<Self> {
		let mut schema = Schema::builder();
		let field_options = TextOptions::default().set_indexing_options(
			TextFieldIndexing::default()
				.set_tokenizer("ngram")
				.set_index_option(IndexRecordOption::WithFreqsAndPositions),
		);

		let id = schema.add_text_field("id", STORED | STRING);
		let scope = schema.add_text_field("scope", field_options.clone());
		let name = schema.add_text_field("name", field_options.clone());
		let description = schema.add_text_field("description", field_options);
		let published_at = schema.add_date_field("published_at", FAST);

		let schema = schema.build();

		let index = Index::create_in_ram(schema);
		index.tokenizers().register(
			"ngram",
			TextAnalyzer::builder(tantivy::tokenizer::NgramTokenizer::all_ngrams(2, 12).unwrap())
				.filter(tantivy::tokenizer::LowerCaser)
				.build(),
		);

		let mut writer = index.writer(WRITER_HEAP_BYTES)?;

		tokio::pin!(packages);
		while let Some((package, package_description)) = packages.try_next().await? {
			writer.add_document(doc!(
				id => package.to_string(),
				scope => package.scope().as_str(),
				name => package.name().as_str(),
				description => package_description,
			))?;
		}
		writer.commit()?;

		let reader = index
			.reader_builder()
			.reload_policy(tantivy::ReloadPolicy::Manual)
			.try_into()?;

		Ok(Self {
			index,
			reader,
			writer: Mutex::new(writer),
			id,
			scope,
			name,
			description,
			published_at,
		})
	}

	pub fn upsert(&self, name: &PackageName, description: &str) -> anyhow::Result<()> {
		let id = name.to_string();
		let mut writer = self.writer.lock().expect("search index writer poisoned");
		writer.delete_term(Term::from_field_text(self.id, &id));
		writer.add_document(doc!(
			self.id => id,
			self.scope => name.scope().as_str(),
			self.name => name.name().as_str(),
			self.description => description,
		))?;
		writer.commit()?;
		drop(writer);

		self.reader.reload()?;
		Ok(())
	}

	pub async fn query(
		&self,
		backend: &dyn Backend,
		query: String,
		limit: usize,
		offset: usize,
	) -> anyhow::Result<QueryResult> {
		let searcher = self.reader.searcher();
		let id_field = self.id;

		let mut query_parser =
			QueryParser::for_index(&self.index, vec![self.scope, self.name, self.description]);
		query_parser.set_field_boost(self.scope, 2.0);
		query_parser.set_field_boost(self.name, 3.5);

		let (count, top_docs) = web::block(move || {
			let (count, top_docs) = if query.is_empty() {
				let (count, top_docs) = searcher.search(
					&AllQuery,
					&(
						Count,
						TopDocs::with_limit(limit)
							.and_offset(offset)
							.order_by_fast_field::<DateTime>("published_at", Order::Desc),
					),
				)?;

				let top_docs = top_docs
					.into_iter()
					.map(|(_, addr)| addr)
					.collect::<Vec<_>>();

				(count, top_docs)
			} else {
				let (count, top_docs) = searcher.search(
					&query_parser.parse_query_lenient(&query).0,
					&(
						Count,
						TopDocs::with_limit(limit)
							.and_offset(offset)
							.order_by_score(),
					),
				)?;

				let top_docs = top_docs
					.into_iter()
					.map(|(score, addr)| {
						let segment_reader = searcher.segment_reader(addr.segment_ord);
						let fast_field_reader =
							segment_reader.fast_fields().date("published_at").unwrap();
						let published_at = fast_field_reader.first(addr.doc_id).unwrap();

						(score, published_at, addr)
					})
					.sorted_by(|a, b| {
						b.0.partial_cmp(&a.0)
							.unwrap_or(std::cmp::Ordering::Equal)
							.then_with(|| b.1.cmp(&a.1))
					})
					.map(|(.., addr)| addr)
					.collect::<Vec<_>>();

				(count, top_docs)
			};

			let top_docs = top_docs
				.into_iter()
				.map(|addr| {
					(&searcher.doc::<HashMap<_, _>>(addr).unwrap()[&id_field])
						.as_str()
						.unwrap()
						.parse::<PackageName>()
						.unwrap()
				})
				.collect::<Vec<_>>();

			Ok::<_, anyhow::Error>((count, top_docs))
		})
		.await??;

		let results = stream::iter(top_docs)
			.then(async |name| {
				let info = backend
					.package_info(&name)
					.await?
					.context("no info for searchable package")?;

				let version = backend
					.package_version(&name, &info.latest_version)
					.await?
					.context("no version for seachable package")?;

				Ok::<_, anyhow::Error>(SearchResultItem {
					package: name,
					version: info.latest_version,
					description: version
						.publish
						.payload
						.into_unsafe_body()
						.payload
						.description
						.into_inner(),
				})
			})
			.try_collect::<Vec<_>>()
			.await?;

		Ok(QueryResult { count, results })
	}
}

// TODO: this uses the repositories of other features, reorganise
