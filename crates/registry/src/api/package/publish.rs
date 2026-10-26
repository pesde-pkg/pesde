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
use pesde::hash::Hash;
use pesde::manifest::Manifest;
use pesde::names::PackageName;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;
use serde::de::DeserializeOwned;
use tokio::io::AsyncReadExt as _;

use crate::AppState;
use crate::api::package::error::Error;
use crate::shared::blob::BlobStorage;
use crate::shared::db::append_leaf;

const MAX_ENTRY_SIZE: usize = 64 * 1024;
const README_FILE_NAME: &str = "README.md";
const MAX_README_SIZE: u64 = 256 * 1024;

#[post("/scope/log/entry")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	mut payload: Multipart,
) -> Result<impl Responder, Error> {
	let mut scope_entry_payload: Option<UserScopeEntryPayload<PublishVersionSignedOpBody>> = None;
	let mut global_entry_payload: Option<ScopeGenesisEntryPayload> = None;
	let mut archive: Option<Bytes> = None;

	while let Some(mut field) = payload.try_next().await? {
		if let Some(field) = FieldExt::new("scope_entry", &mut field) {
			scope_entry_payload = Some(field.json(MAX_ENTRY_SIZE).await?)
		} else if let Some(field) = FieldExt::new("global_entry", &mut field) {
			global_entry_payload = Some(field.json(MAX_ENTRY_SIZE).await?);
		} else if let Some(field) = FieldExt::new("archive", &mut field) {
			archive = Some(field.bytes(app_state.max_archive_size).await?);
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

	Ok(HttpResponse::Created().finish())
}

struct FieldExt<'a> {
	field: &'a mut Field,
	name: &'static str,
}
impl<'a> FieldExt<'a> {
	fn new(name: &'static str, field: &'a mut Field) -> Option<Self> {
		field
			.name()
			.is_some_and(|n| n == name)
			.then_some(Self { field, name })
	}

	async fn bytes(&mut self, limit: usize) -> Result<Bytes, Error> {
		self.field
			.bytes(limit)
			.map_err(|_| Error::FieldTooLarge {
				name: self.name,
				limit,
			})
			.await?
			.map_err(Into::into)
	}

	async fn json<T: DeserializeOwned>(&mut self, limit: usize) -> Result<T, Error> {
		let bytes = self.bytes(limit).await?;

		serde_json::from_slice(&bytes).map_err(Into::into)
	}
}

async fn handler(
	db: &dyn Backend,
	blob: &BlobStorage,
	scope_entry_payload: UserScopeEntryPayload<PublishVersionSignedOpBody>,
	global_entry_payload: Option<ScopeGenesisEntryPayload>,
	archive: Bytes,
) -> Result<(), Error> {
	let scope_entry_payload = scope_entry_payload.into_inner();
	let op_payload = &scope_entry_payload.body.op_payload;
	let scope_id = &op_payload.scope_id;

	// let mut (scope_size, scope_tx) = match global_entry_payload {
	// 	Some(payload) => {
	// 		let () = db.begin_write_creating(scope_id).await?;
	// 	}
	// };

	let op_hash = &op_payload.op.archive_hash;
	if Hash::digest(op_hash.algorithm(), &archive) != *op_hash {
		return Err(Error::ArchiveHashMismatch);
	}

	let tempdir = tokio::task::spawn_blocking(tempfile::tempdir)
		.await
		.context("failed to spawn tempdir creation")?
		.context("failed to create tempdir")?;

	tokio_tar::Archive::new(async_compression::tokio::bufread::ZstdDecoder::new(
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
		|| manifest.name.local_name() != &body.payload.name
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

	todo!();
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
