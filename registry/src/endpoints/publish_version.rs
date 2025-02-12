use crate::{
	auth::UserId,
	error::{ErrorResponse, RegistryError},
	git::push_changes,
	package::{read_package, read_scope_info},
	search::update_search_version,
	storage::StorageImpl,
	AppState,
};
use actix_web::{web, web::Bytes, HttpResponse};
use async_compression::Level;
use convert_case::{Case, Casing};
use fs_err::tokio as fs;
use pesde::{
	manifest::{DependencyType, Manifest},
	source::{
		git_index::GitBasedSource,
		ids::VersionId,
		pesde::{DocEntry, DocEntryKind, IndexFileEntry, ScopeInfo, SCOPE_INFO_FILE},
		specifiers::DependencySpecifiers,
		traits::RefreshOptions,
		ADDITIONAL_FORBIDDEN_FILES, IGNORED_DIRS, IGNORED_FILES,
	},
	MANIFEST_FILE_NAME,
};
use sentry::add_breadcrumb;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
	collections::{BTreeSet, HashMap},
	io::Cursor,
};
use tokio::{
	io::{AsyncReadExt, AsyncWriteExt},
	task::JoinSet,
};

#[derive(Debug, Deserialize, Default)]
struct DocEntryInfo {
	#[serde(default)]
	label: Option<String>,
	#[serde(default, alias = "position")]
	sidebar_position: Option<usize>,
	#[serde(default)]
	collapsed: bool,
}

pub async fn publish_package(
	app_state: web::Data<AppState>,
	bytes: Bytes,
	user_id: web::ReqData<UserId>,
) -> Result<HttpResponse, RegistryError> {
	let source = app_state.source.write().await;
	source
		.refresh(&RefreshOptions {
			project: app_state.project.clone(),
		})
		.await
		.map_err(Box::new)?;
	let config = source.config(&app_state.project).await?;

	let package_dir = tempfile::tempdir()?;

	{
		let mut decoder = async_compression::tokio::bufread::GzipDecoder::new(Cursor::new(&bytes));
		let mut archive = tokio_tar::Archive::new(&mut decoder);

		archive.unpack(package_dir.path()).await?;
	}

	let mut manifest = None::<Manifest>;
	let mut readme = None::<Vec<u8>>;
	let mut docs = BTreeSet::new();
	let mut docs_pages = HashMap::new();

	let mut read_dir = fs::read_dir(package_dir.path()).await?;
	while let Some(entry) = read_dir.next_entry().await? {
		let file_name = entry
			.file_name()
			.to_str()
			.ok_or_else(|| {
				RegistryError::InvalidArchive("file name contains non UTF-8 characters".into())
			})?
			.to_string();

		if entry.file_type().await?.is_dir() {
			if IGNORED_DIRS.contains(&file_name.as_str()) {
				return Err(RegistryError::InvalidArchive(format!(
					"archive contains forbidden directory: {file_name}"
				)));
			}

			if file_name == "docs" {
				let mut stack = vec![(
					BTreeSet::new(),
					fs::read_dir(entry.path()).await?,
					None::<DocEntryInfo>,
				)];

				'outer: while let Some((set, iter, category_info)) = stack.last_mut() {
					while let Some(entry) = iter.next_entry().await? {
						let file_name = entry
							.file_name()
							.to_str()
							.ok_or_else(|| {
								RegistryError::InvalidArchive(
									"file name contains non UTF-8 characters".into(),
								)
							})?
							.to_string();

						if entry.file_type().await?.is_dir() {
							stack.push((
								BTreeSet::new(),
								fs::read_dir(entry.path()).await?,
								Some(DocEntryInfo {
									label: Some(file_name.to_case(Case::Title)),
									..Default::default()
								}),
							));
							continue 'outer;
						}

						if file_name == "_category_.json" {
							let info = fs::read_to_string(entry.path()).await?;
							let mut info: DocEntryInfo = serde_json::from_str(&info)?;
							let old_info = category_info.take();
							info.label = info.label.or(old_info.and_then(|i| i.label));
							*category_info = Some(info);
							continue;
						}

						let Some(file_name) = file_name.strip_suffix(".md") else {
							continue;
						};

						let content = fs::read_to_string(entry.path()).await?;
						let content = content.trim();
						let hash = format!("{:x}", Sha256::digest(content.as_bytes()));

						let mut gz = async_compression::tokio::bufread::GzipEncoder::with_quality(
							Cursor::new(content.as_bytes().to_vec()),
							Level::Best,
						);
						let mut bytes = vec![];
						gz.read_to_end(&mut bytes).await?;
						docs_pages.insert(hash.to_string(), bytes);

						let mut lines = content.lines().peekable();
						let front_matter = if lines.peek().filter(|l| **l == "---").is_some() {
							lines.next(); // skip the first `---`

							let front_matter = lines
								.by_ref()
								.take_while(|l| *l != "---")
								.collect::<Vec<_>>()
								.join("\n");

							lines.next(); // skip the last `---`

							front_matter
						} else {
							"".to_string()
						};

						let h1 = lines
							.find(|l| !l.trim().is_empty())
							.and_then(|l| l.strip_prefix("# "))
							.map(|s| s.to_string());

						let info: DocEntryInfo =
							serde_yaml::from_str(&front_matter).map_err(|_| {
								RegistryError::InvalidArchive(format!(
									"doc {file_name}'s frontmatter isn't valid YAML"
								))
							})?;

						set.insert(DocEntry {
							label: info.label.or(h1).unwrap_or(file_name.to_case(Case::Title)),
							position: info.sidebar_position,
							kind: DocEntryKind::Page {
								name: entry
									.path()
									.strip_prefix(package_dir.path().join("docs"))
									.unwrap()
									.with_extension("")
									.to_str()
									.ok_or_else(|| {
										RegistryError::InvalidArchive(
											"file name contains non UTF-8 characters".into(),
										)
									})?
									// ensure that the path is always using forward slashes
									.replace("\\", "/"),
								hash,
							},
						});
					}

					// should never be None
					let (popped, _, category_info) = stack.pop().unwrap();
					docs = popped;

					if let Some((set, _, _)) = stack.last_mut() {
						let category_info = category_info.unwrap_or_default();

						set.insert(DocEntry {
							label: category_info.label.unwrap(),
							position: category_info.sidebar_position,
							kind: DocEntryKind::Category {
								items: {
									let curr_docs = docs;
									docs = BTreeSet::new();
									curr_docs
								},
								collapsed: category_info.collapsed,
							},
						});
					}
				}
			}

			continue;
		}

		if IGNORED_FILES.contains(&file_name.as_str())
			|| ADDITIONAL_FORBIDDEN_FILES.contains(&file_name.as_str())
		{
			return Err(RegistryError::InvalidArchive(format!(
				"archive contains forbidden file: {file_name}"
			)));
		}

		if file_name == MANIFEST_FILE_NAME {
			let content = fs::read_to_string(entry.path()).await?;

			manifest = Some(toml::de::from_str(&content)?);
		} else if file_name
			.to_lowercase()
			.split_once('.')
			.filter(|(file, ext)| *file == "readme" && (*ext == "md" || *ext == "txt"))
			.is_some()
		{
			if readme.is_some() {
				return Err(RegistryError::InvalidArchive(
					"archive contains multiple readme files".into(),
				));
			}

			let mut file = fs::File::open(entry.path()).await?;

			let mut gz = async_compression::tokio::write::GzipEncoder::new(vec![]);
			tokio::io::copy(&mut file, &mut gz).await?;
			gz.shutdown().await?;
			readme = Some(gz.into_inner());
		}
	}

	let Some(manifest) = manifest else {
		return Err(RegistryError::InvalidArchive(
			"archive doesn't contain a manifest".into(),
		));
	};

	add_breadcrumb(sentry::Breadcrumb {
		category: Some("publish".into()),
		message: Some(format!(
			"publish request for {}@{} {}. has readme: {}. docs: {}",
			manifest.name,
			manifest.version,
			manifest.target,
			readme.is_some(),
			docs_pages.len()
		)),
		level: sentry::Level::Info,
		..Default::default()
	});

	{
		let dependencies = manifest.all_dependencies().map_err(|e| {
			RegistryError::InvalidArchive(format!("manifest has invalid dependencies: {e}"))
		})?;

		for (specifier, ty) in dependencies.values() {
			// we need not verify dev dependencies, as they won't be installed
			if *ty == DependencyType::Dev {
				continue;
			}

			match specifier {
				DependencySpecifiers::Pesde(specifier) => {
					let allowed = match gix::Url::try_from(&*specifier.index) {
						Ok(url) => config
							.other_registries_allowed
							.is_allowed_or_same(source.repo_url().clone(), url),
						Err(_) => false,
					};

					if !allowed {
						return Err(RegistryError::InvalidArchive(format!(
							"invalid index in pesde dependency {specifier}"
						)));
					}
				}
				DependencySpecifiers::Wally(specifier) => {
					let allowed = match gix::Url::try_from(&*specifier.index) {
						Ok(url) => config.wally_allowed.is_allowed(url),
						Err(_) => false,
					};

					if !allowed {
						return Err(RegistryError::InvalidArchive(format!(
							"invalid index in wally dependency {specifier}"
						)));
					}
				}
				DependencySpecifiers::Git(specifier) => {
					if !config.git_allowed.is_allowed(specifier.repo.clone()) {
						return Err(RegistryError::InvalidArchive(
							"git dependencies are not allowed".into(),
						));
					}
				}
				DependencySpecifiers::Workspace(_) => {
					// workspace specifiers are to be transformed into pesde specifiers by the sender
					return Err(RegistryError::InvalidArchive(
						"non-transformed workspace dependency".into(),
					));
				}
				DependencySpecifiers::Path(_) => {
					return Err(RegistryError::InvalidArchive(
						"path dependencies are not allowed".into(),
					));
				}
			}
		}

		let mut files = HashMap::new();

		let scope = read_scope_info(&app_state, manifest.name.scope(), &source).await?;
		match scope {
			Some(info) => {
				if !info.owners.contains(&user_id.0) {
					return Ok(HttpResponse::Forbidden().finish());
				}
			}
			None => {
				let scope_info = toml::to_string(&ScopeInfo {
					owners: BTreeSet::from([user_id.0]),
				})?;

				files.insert(SCOPE_INFO_FILE.to_string(), scope_info.into_bytes());
			}
		}

		let mut file = read_package(&app_state, &manifest.name, &source)
			.await?
			.unwrap_or_default();

		let new_entry = IndexFileEntry {
			target: manifest.target.clone(),
			published_at: jiff::Timestamp::now(),
			engines: manifest.engines.clone(),
			description: manifest.description.clone(),
			license: manifest.license.clone(),
			authors: manifest.authors.clone(),
			repository: manifest.repository.clone(),
			yanked: false,
			docs,

			dependencies,
		};

		let same_version = file
			.entries
			.iter()
			.find(|(v_id, _)| *v_id.version() == manifest.version);
		if let Some((_, other_entry)) = same_version {
			// description cannot be different - which one to render in the "Recently published" list?
			if other_entry.description != new_entry.description {
				return Ok(HttpResponse::BadRequest().json(ErrorResponse {
					error: "same versions with different descriptions are forbidden".to_string(),
				}));
			}
		}

		if file
			.entries
			.insert(
				VersionId::new(manifest.version.clone(), manifest.target.kind()),
				new_entry.clone(),
			)
			.is_some()
		{
			return Ok(HttpResponse::Conflict().finish());
		}

		files.insert(
			manifest.name.name().to_string(),
			toml::to_string(&file)?.into_bytes(),
		);

		push_changes(
			&app_state,
			&source,
			manifest.name.scope().to_string(),
			files,
			format!(
				"add {}@{} {}",
				manifest.name, manifest.version, manifest.target
			),
		)
		.await?;

		update_search_version(&app_state, &manifest.name, &new_entry);
	}

	let version_id = VersionId::new(manifest.version.clone(), manifest.target.kind());

	let mut tasks = docs_pages
		.into_iter()
		.map(|(hash, content)| {
			let app_state = app_state.clone();
			async move { app_state.storage.store_doc(hash, content).await }
		})
		.collect::<JoinSet<_>>();

	{
		let app_state = app_state.clone();
		let name = manifest.name.clone();
		let version_id = version_id.clone();

		tasks.spawn(async move {
			app_state
				.storage
				.store_package(&name, &version_id, bytes.to_vec())
				.await
		});
	}

	if let Some(readme) = readme {
		let app_state = app_state.clone();
		let name = manifest.name.clone();
		let version_id = version_id.clone();

		tasks.spawn(async move {
			app_state
				.storage
				.store_readme(&name, &version_id, readme)
				.await
		});
	}

	while let Some(res) = tasks.join_next().await {
		res.unwrap()?;
	}

	Ok(HttpResponse::Ok().body(format!("published {}@{version_id}", manifest.name)))
}
