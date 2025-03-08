use crate::AppState;
use pesde::{
	names::PackageName,
	source::{
		git_index::{root_tree, GitBasedSource as _},
		ids::VersionId,
		pesde::{IndexFile, IndexFileEntry, PesdePackageSource, SCOPE_INFO_FILE},
	},
	Project,
};
use std::collections::BTreeMap;
use tantivy::{
	doc,
	query::QueryParser,
	schema::{IndexRecordOption, TextFieldIndexing, TextOptions, FAST, STORED, STRING},
	tokenizer::TextAnalyzer,
	DateTime, IndexReader, IndexWriter, Term,
};

type Entries = BTreeMap<String, gix::ObjectId>;

struct TreeIterator<'repo> {
	repo: &'repo gix::Repository,
	entries: Entries,
	current: Option<(String, Entries)>,
}

fn collect_entries(tree: &gix::Tree) -> Result<Entries, gix::objs::decode::Error> {
	tree.iter()
		.map(|res| res.map(|r| (r.filename().to_string(), r.object_id())))
		.collect()
}

impl Iterator for TreeIterator<'_> {
	type Item = (PackageName, IndexFile);

	fn next(&mut self) -> Option<Self::Item> {
		if self
			.current
			.as_ref()
			.is_none_or(|(_, entries)| entries.is_empty())
		{
			loop {
				let (scope_name, scope_oid) = self.entries.pop_last()?;

				let object = self
					.repo
					.find_object(scope_oid)
					.expect("failed to get scope object");

				if object.kind != gix::objs::Kind::Tree {
					continue;
				}

				let tree = object.into_tree();
				let mut entries = collect_entries(&tree).expect("failed to read scope entries");

				entries.remove(SCOPE_INFO_FILE);

				if entries.is_empty() {
					continue;
				}

				self.current = Some((scope_name, entries));
				break;
			}
		}

		let (scope_name, entries) = self.current.as_mut()?;
		let (file_name, file_oid) = entries.pop_last()?;

		let object = self
			.repo
			.find_object(file_oid)
			.expect("failed to get scope entry object");

		if object.kind != gix::objs::Kind::Blob {
			return None;
		}

		let mut blob = object.into_blob();
		let string = String::from_utf8(blob.take_data()).expect("failed to parse utf8");

		let file = toml::from_str(&string).expect("failed to parse index file");

		Some((
			// if this panics, it's an issue with the index.
			format!("{scope_name}/{file_name}").parse().unwrap(),
			file,
		))
	}
}

pub fn find_max_searchable(file: &IndexFile) -> Option<(&VersionId, &IndexFileEntry)> {
	file.entries
		.iter()
		.filter(|(_, entry)| !entry.yanked)
		.max_by(|(v_id_a, entry_a), (v_id_b, entry_b)| {
			v_id_a
				.version()
				.cmp(v_id_b.version())
				.then(entry_a.published_at.cmp(&entry_b.published_at))
		})
}

pub async fn make_search(
	project: &Project,
	source: &PesdePackageSource,
) -> (IndexReader, IndexWriter, QueryParser) {
	let mut schema_builder = tantivy::schema::SchemaBuilder::new();

	let field_options = TextOptions::default().set_indexing_options(
		TextFieldIndexing::default()
			.set_tokenizer("ngram")
			.set_index_option(IndexRecordOption::WithFreqsAndPositions),
	);

	let id_field = schema_builder.add_text_field("id", STRING | STORED);

	let scope = schema_builder.add_text_field("scope", field_options.clone());
	let name = schema_builder.add_text_field("name", field_options.clone());
	let description = schema_builder.add_text_field("description", field_options);
	let published_at = schema_builder.add_date_field("published_at", FAST);

	let search_index = tantivy::Index::create_in_ram(schema_builder.build());
	search_index.tokenizers().register(
		"ngram",
		TextAnalyzer::builder(tantivy::tokenizer::NgramTokenizer::all_ngrams(1, 12).unwrap())
			.filter(tantivy::tokenizer::LowerCaser)
			.build(),
	);

	let search_reader = search_index
		.reader_builder()
		.reload_policy(tantivy::ReloadPolicy::Manual)
		.try_into()
		.unwrap();
	let mut search_writer = search_index.writer(50_000_000).unwrap();

	let path = source.path(project);
	let repo = gix::open(path).expect("failed to open index");
	let tree = root_tree(&repo).expect("failed to get root tree");

	let iter = TreeIterator {
		entries: collect_entries(&tree).expect("failed to read entries"),
		repo: &repo,
		current: None,
	};

	for (pkg_name, file) in iter {
		if !file.meta.deprecated.is_empty() {
			continue;
		}

		let Some((_, latest_entry)) = find_max_searchable(&file) else {
			continue;
		};

		search_writer
			.add_document(doc!(
				id_field => pkg_name.to_string(),
				scope => pkg_name.scope(),
				name => pkg_name.name(),
				description => latest_entry.description.clone().unwrap_or_default(),
				published_at => DateTime::from_timestamp_nanos(latest_entry.published_at.as_nanosecond() as i64),
			))
			.unwrap();
	}

	search_writer.commit().unwrap();
	search_reader.reload().unwrap();

	let mut query_parser = QueryParser::for_index(&search_index, vec![scope, name, description]);
	query_parser.set_field_boost(scope, 2.0);
	query_parser.set_field_boost(name, 3.5);

	(search_reader, search_writer, query_parser)
}

pub fn update_search_version(app_state: &AppState, name: &PackageName, entry: &IndexFileEntry) {
	let mut search_writer = app_state.search_writer.lock().unwrap();
	let schema = search_writer.index().schema();
	let id_field = schema.get_field("id").unwrap();

	search_writer.delete_term(Term::from_field_text(id_field, &name.to_string()));

	search_writer.add_document(doc!(
        id_field => name.to_string(),
        schema.get_field("scope").unwrap() => name.scope(),
        schema.get_field("name").unwrap() => name.name(),
        schema.get_field("description").unwrap() => entry.description.clone().unwrap_or_default(),
        schema.get_field("published_at").unwrap() => DateTime::from_timestamp_nanos(entry.published_at.as_nanosecond() as i64)
    )).unwrap();

	search_writer.commit().unwrap();
	drop(search_writer);
	app_state.search_reader.reload().unwrap();
}

pub fn search_version_changed(app_state: &AppState, name: &PackageName, file: &IndexFile) {
	let entry = if file.meta.deprecated.is_empty() {
		find_max_searchable(file)
	} else {
		None
	};

	let Some((_, entry)) = entry else {
		let mut search_writer = app_state.search_writer.lock().unwrap();
		let schema = search_writer.index().schema();
		let id_field = schema.get_field("id").unwrap();

		search_writer.delete_term(Term::from_field_text(id_field, &name.to_string()));
		search_writer.commit().unwrap();
		drop(search_writer);
		app_state.search_reader.reload().unwrap();

		return;
	};

	update_search_version(app_state, name, entry);
}
