use std::collections::BTreeMap;
use std::io::Cursor;

use actix_multipart::Field;
use actix_multipart::Multipart;
use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::post;
use actix_web::web;
use actix_web::web::Bytes;
use anyhow::Context as _;
use fs_err::tokio as fs;
use futures::TryFutureExt as _;
use futures::TryStreamExt as _;
use pesde::MANIFEST_FILE_NAME;
use pesde::bounded::Bounded;
use pesde::hash::Hash;
use pesde::manifest::Manifest;
use pesde::names::PackageName;
use pesde::source::DependencySpecifiers;
use pesde::source::pesde::registry::*;
use pesde::source::pesde::specifier::RegistryPesdeDependencySpecifier;
use pesde::source::wally::specifier::RegistryWallyDependencySpecifier;
use serde::de::DeserializeOwned;
use tokio::io::AsyncReadExt as _;

use crate::AppState;
use crate::features::package::Error;
use crate::shared::auth::WriteGuard;
use crate::shared::blob::BlobStorage;
use crate::shared::db::Backend;
use crate::shared::db::ScopeControl;
use crate::shared::db::append_leaf;

const MAX_ENTRY_SIZE: usize = 64 * 1024;
const README_FILE_NAME: &str = "README.md";
const MAX_README_SIZE: u64 = 256 * 1024;

#[post("/package/publish")]
pub(super) async fn http_v2(
	_access_guard: WriteGuard,
	app_state: web::Data<AppState>,
	mut payload: Multipart,
) -> Result<impl Responder, Error> {
	let mut entry: Option<PublishScopeEntry> = None;
	let mut scope_entry: Option<ManifestUpdateScopeEntry> = None;
	let mut archive: Option<Bytes> = None;

	while let Some(mut field) = payload.try_next().await.map_err(|e| bad_multipart(&e))? {
		match field.name() {
			Some("entry") => entry = Some(json_field(&mut field, MAX_ENTRY_SIZE, "entry").await?),
			Some("scope") => {
				scope_entry = Some(json_field(&mut field, MAX_ENTRY_SIZE, "scope").await?);
			}
			Some("archive") => {
				let limit = app_state.max_archive_size;
				archive = Some(
					field
						.bytes(limit)
						.await
						.map_err(|_e| field_too_large("archive", limit))?
						.map_err(|e| bad_multipart(&e))?,
				);
			}
			_ => {}
		}
	}

	let entry = entry.ok_or_else(|| Error::BadRequest("missing `entry` field".to_string()))?;
	let archive =
		archive.ok_or_else(|| Error::BadRequest("missing `archive` field".to_string()))?;

	let package = {
		let body = entry.unsafe_body();
		PackageName::new(body.scope.clone(), body.payload.name.clone())
	};

	handler(
		app_state.db.as_ref(),
		&app_state.blob_storage,
		entry,
		scope_entry,
		archive,
	)
	.await?;

	if let Err(e) = app_state
		.search
		.update(app_state.db.as_ref(), package)
		.await
	{
		tracing::error!("failed to index published package for search: {e:#?}");
	}

	Ok(HttpResponse::Ok().finish())
}

async fn json_field<T: DeserializeOwned>(
	field: &mut Field,
	limit: usize,
	name: &str,
) -> Result<T, Error> {
	let bytes = field
		.bytes(limit)
		.await
		.map_err(|_e| field_too_large(name, limit))?
		.map_err(|e| bad_multipart(&e))?;

	serde_json::from_slice(&bytes)
		.map_err(|e| Error::BadRequest(format!("invalid `{name}` field: {e}")))
}

fn field_too_large(name: &str, limit: usize) -> Error {
	Error::BadRequest(format!(
		"`{name}` field exceeds the maximum size of {limit} bytes"
	))
}

fn bad_multipart(e: &actix_multipart::MultipartError) -> Error {
	Error::BadRequest(format!("invalid multipart request: {e}"))
}

async fn handler(
	db: &dyn Backend,
	blob: &BlobStorage,
	entry: PublishScopeEntry,
	scope_entry: Option<ManifestUpdateScopeEntry>,
	archive: Bytes,
) -> Result<(), Error> {
	let mut store = db.begin_write().await?;

	let author = db
		.author_key(&mut store, &entry.unsafe_body().author_identity)
		.await?
		.ok_or(Error::UnknownIdentity)?;
	let Some((sig, body)) = entry.into_verified_external(&author.key) else {
		return Err(Error::InvalidSignature);
	};

	if Hash::from_bytes(body.payload.archive_hash.algorithm(), &archive)
		!= body.payload.archive_hash
	{
		return Err(Error::ArchiveHashMismatch);
	}

	let tempdir = tokio::task::spawn_blocking(tempfile::tempdir)
		.await
		.context("failed to spawn tempdir creation")?
		.context("failed to create tempdir")?;

	async_tar::Archive::new(async_compression::tokio::bufread::GzipDecoder::new(
		&*archive.clone(),
	))
	.unpack(tempdir.path())
	.await
	.map_err(|e| Error::BadRequest(format!("invalid archive: {e}")))?;

	let manifest = fs::read_to_string(tempdir.path().join(MANIFEST_FILE_NAME))
		.await
		.map_err(|e| Error::BadRequest(format!("could not read {MANIFEST_FILE_NAME}: {e}")))?;
	let manifest: Manifest = toml::from_str(&manifest)
		.map_err(|e| Error::BadRequest(format!("invalid {MANIFEST_FILE_NAME}: {e}")))?;

	let readme = match fs::File::open(tempdir.path().join(README_FILE_NAME)).await {
		Ok(file) => {
			let mut buffer = Vec::new();
			file.take(MAX_README_SIZE + 1)
				.read_to_end(&mut buffer)
				.await
				.map_err(|e| Error::Internal(e.into()))?;
			if buffer.len() as u64 > MAX_README_SIZE {
				return Err(Error::BadRequest(format!(
					"{README_FILE_NAME} exceeds the maximum size of {MAX_README_SIZE} bytes"
				)));
			}
			Some(buffer)
		}
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
		Err(e) => return Err(Error::Internal(e.into())),
	};

	tokio::task::spawn_blocking(move || tempdir.close())
		.await
		.unwrap()
		.map_err(anyhow::Error::from)
		.map_err(Error::Internal)?;

	if manifest.private {
		return Err(Error::BadRequest(
			"cannot publish a private package".to_string(),
		));
	}

	if manifest.name.scope() != &body.scope
		|| manifest.name.name() != &body.payload.name
		|| *manifest.version != *body.payload.version
		|| *manifest.description != *body.payload.description
		|| *manifest.license != *body.payload.license
		|| manifest.repository.as_deref() != body.payload.repository.as_deref()
		|| *manifest.authors != *body.payload.authors
	{
		return Err(Error::BadRequest(
			"the manifest does not match the entry".to_string(),
		));
	}

	let manifest_deps = manifest
		.all_dependencies()
		.map_err(|e| Error::BadRequest(format!("invalid manifest dependencies: {e}")))?;
	if manifest_deps.len() != body.payload.dependencies.len() {
		return Err(Error::BadRequest(
			"the manifest dependencies do not match the entry".to_string(),
		));
	}
	for (alias, (manifest_spec, manifest_ty)) in &manifest_deps {
		let Some((entry_spec, entry_ty)) = body.payload.dependencies.get(alias) else {
			return Err(Error::BadRequest(format!(
				"dependency `{alias}` is missing from the entry"
			)));
		};

		let manifest_spec = match manifest_spec {
			DependencySpecifiers::Pesde(s) => {
				RegistryDependencySpecifier::Pesde(RegistryPesdeDependencySpecifier {
					name: s.name.clone(),
					version: Bounded::new(s.version.clone()).map_err(|_e| {
						Error::BadRequest(format!(
							"invalid pesde dependency `{alias}` version value"
						))
					})?,
					registry: Some(&*s.registry)
						.filter(|r| !r.is_empty())
						.map(|r| {
							r.parse().map_err(|_e| {
								Error::BadRequest(format!(
									"invalid pesde dependency `{alias}` registry value"
								))
							})
						})
						.transpose()?,
					realm: s.realm,
				})
			}
			DependencySpecifiers::Wally(s) => {
				RegistryDependencySpecifier::Wally(RegistryWallyDependencySpecifier {
					name: s.name.clone(),
					version: Bounded::new(s.version.clone()).map_err(|_e| {
						Error::BadRequest(format!(
							"invalid wally dependency `{alias}` version value"
						))
					})?,
					index: s.index.parse().map_err(|_e| {
						Error::BadRequest(format!(
							"invalid wally dependency `{alias}` registry value"
						))
					})?,
					realm: s.realm,
				})
			}
			_ => {
				return Err(Error::BadRequest(format!(
					"dependency `{alias}` is not a registry dependency"
				)));
			}
		};

		if *entry_spec != manifest_spec || entry_ty != manifest_ty {
			return Err(Error::BadRequest(format!(
				"dependency `{alias}` does not match the manifest"
			)));
		}
	}

	let Some(access) = db
		.authorize_scope_write(
			&mut store,
			&body.scope,
			&author,
			ScopeControl::PublishOrCreate(&body.payload.name),
		)
		.await?
	else {
		return Err(Error::Unauthorized);
	};

	let (store, publish_pos) = if access.scope_exists {
		(store, access.pos)
	} else {
		let Some(scope_entry) = scope_entry else {
			return Err(Error::BadRequest(
				"scope does not exist; a signed scope manifest must accompany the publish"
					.to_string(),
			));
		};

		if scope_entry.unsafe_body().scope != body.scope
			|| scope_entry.unsafe_body().author_identity != body.author_identity
		{
			return Err(Error::BadRequest(
				"the scope creation entry must be authored by the publisher for the same scope"
					.to_string(),
			));
		}
		let Some((scope_sig, scope_body)) = scope_entry.into_verified_external(&author.key) else {
			return Err(Error::InvalidSignature);
		};
		let expected_manifest = ScopeManifest {
			owner: body.author_identity,
			members: BTreeMap::new(),
		};
		if scope_body.payload.manifest != expected_manifest {
			return Err(Error::BadRequest(
				"a new scope's manifest must name the publisher as its sole owner".to_string(),
			));
		}

		let (mut store, publish_pos) = append_leaf(store, access.pos, &scope_body).await?;
		db.insert_manifest_update(&mut store, access.pos, &scope_sig, &scope_body)
			.await?;
		(store, publish_pos)
	};

	let (mut store, _) = append_leaf(store, publish_pos, &body).await?;
	db.insert_publish(&mut store, publish_pos, &sig, &body)
		.await?;

	let package_name = PackageName::new(body.scope.clone(), body.payload.name.clone());
	tokio::try_join!(
		blob.put_package_archive(&package_name, &body.payload.version, Cursor::new(archive))
			.map_err(Error::Internal),
		async {
			if let Some(readme) = readme {
				blob.put_package_readme(&package_name, &body.payload.version, Cursor::new(readme))
					.await
					.map_err(Error::Internal)?;
			}

			Ok(())
		},
	)?;
	store.commit().await?;

	Ok(())
}
