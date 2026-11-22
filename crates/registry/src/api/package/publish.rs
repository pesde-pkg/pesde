use std::collections::HashSet;
use std::num::NonZero;
use std::str::FromStr as _;

use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::post;
use actix_web::web;
use actix_web::web::Bytes;
use pesde::MANIFEST_FILE_NAME;
use pesde::hash::Hash;
use pesde::registry::Canonical as _;
use pesde::registry::CurrentHash;
use pesde::registry::Entry;
use pesde::registry::LocalNameId;
use pesde::registry::PackageVersionValue;
use pesde::registry::PublishPackageBody;
use pesde::registry::PublishVersionUserScopeOpBody;
use pesde::registry::ScopeEntry;
use pesde::registry::ScopeEntryPayload;
use pesde::registry::ScopeGenesisPayload;
use pesde::registry::ScopeId;
use pesde::registry::ScopeState;
use pesde::registry::Signed;
use pesde::registry::UnvalidatedSigned;
use pesde::registry::UserScopeOp;
use pesde::registry::manifest::ExportPath;
use pesde::registry::manifest::ManifestPackage;
use pesde::registry::manifest::ResolvedManifest;
use pesde_registry_core::db::Backend;
use pesde_registry_core::db::CreatingScopeLockResult;
use pesde_registry_core::db::ExistingScopeLockResult;
use pesde_registry_core::db::PermissionWidth;
use pesde_registry_core::features::global::Global;
use pesde_registry_core::features::scope::Scoped;
use tokio::try_join;

use crate::AppState;
use crate::Db;
use crate::api::package::error::Error;
use crate::shared::blob::BlobStorage;
use crate::shared::db::run_tx;
use crate::shared::log::append_leaf;
use crate::shared::microsecond_timestamp;
use crate::shared::tree::run_tree;

const JSON_BODY_LIMIT: usize = 1024 * 1024;

#[post("/package/publish")]
pub(super) async fn http_v2(
	db: Db,
	app_state: web::Data<AppState>,
	payload: web::Payload,
) -> Result<impl Responder, Error> {
	let bytes_result = payload
		.to_bytes_limited(app_state.max_archive_size + JSON_BODY_LIMIT)
		.await?;

	let mut bytes = match bytes_result {
		Ok(bytes) => bytes,
		Err(e) => return Ok(e.error_response()),
	};

	let separator_idx = bytes
		.iter()
		.position(|&b| b == b'\n')
		.ok_or(Error::MalformedBody)?;

	let archive_bytes = bytes.split_off(separator_idx + 1);

	let body: PublishPackageBody =
		serde_json::from_slice(&bytes[..separator_idx]).map_err(Error::MalformedJson)?;

	let entry = handler(db.as_ref(), &app_state.blob_storage, body, archive_bytes).await?;

	// if let Err(e) = app_state
	// 	.search
	// 	.update(app_state.db.as_ref(), package)
	// 	.await
	// {
	// 	tracing::error!("failed to index published package for search: {e:#?}");
	// }

	Ok(HttpResponse::Created().json(entry))
}

async fn handler(
	db: &dyn Backend,
	blob: &BlobStorage,
	body: PublishPackageBody,
	archive: Bytes,
) -> Result<ScopeEntry, Error> {
	let PublishPackageBody {
		scope_genesis,
		publish_version,
	} = body;

	let UnvalidatedSigned {
		sig: _,
		body:
			PublishVersionUserScopeOpBody {
				header,
				package_version,
				archive_hash,
				manifest_hash,
			},
	} = publish_version.inner();

	if Hash::digest(archive_hash.algorithm(), &archive) != *archive_hash {
		return Err(Error::HashMismatch);
	}

	let mut zip =
		async_zip::base::read::seek::ZipFileReader::with_tokio(std::io::Cursor::new(&*archive))
			.await?;

	if zip.file().zip64() {
		// zip64 is an overall nightmare of a format. while async-zip does support it,
		// we don't want to support it in the registry protocol. we can always lift this restriction later if we need to,
		// but package archives shouldn't be that big in the first place
		return Err(Error::Zip64NotAllowed);
	}

	let manifest_file_idx = {
		let mut manifest_file = None;

		// check for duplicate paths (ZIP allows them) and case-only differences
		let mut seen_paths = HashSet::new();
		for (idx, c) in zip.file().entries().iter().enumerate() {
			match c.filename().as_str() {
				Ok(p)
					if let p = p.strip_suffix('/').unwrap_or(p)
						&& ExportPath::from_str(p).is_ok()
						&& seen_paths.insert(p.to_lowercase().into_boxed_str()) => {}
				_ => {
					let bytes: Box<[u8]> = c.filename().as_bytes().into();

					return Err(Error::InvalidPath(bytes.into()));
				}
			}

			// specifically case-sensitive, this isn't an oversight!
			if c.filename().as_bytes() == MANIFEST_FILE_NAME.as_bytes() {
				manifest_file = Some(idx);
			}
		}

		manifest_file.ok_or(Error::MalformedBody)?
	};

	let mut manifest_file = zip.reader_with_entry(manifest_file_idx).await?;
	let mut manifest_contents = String::new();

	manifest_file
		.read_to_string_checked(&mut manifest_contents)
		.await
		.map_err(Error::InvalidZip)?;

	if Hash::digest(manifest_hash.algorithm(), &manifest_contents) != *manifest_hash {
		return Err(Error::HashMismatch);
	}

	let manifest: ResolvedManifest =
		toml::from_str(&manifest_contents).map_err(Error::InvalidManifest)?;

	{
		let ResolvedManifest {
			package:
				ManifestPackage {
					name,
					version: _,
					description: _,
					// TODO: validate license as SPDX
					license: _,
					repository: _,
				},
			exports: _,
			config_luau: _,
			// TODO: validate allowed foreign registries, indices, etc.
			dependencies: _,
		} = &manifest;

		if ScopeId::from(name.scope()) != header.scope_id
			|| LocalNameId::from(name.local_name()) != package_version.pkg
			|| manifest.package.version != package_version.version
		{
			return Err(Error::ManifestStateMismatch);
		}
	}

	let (tx, scope_size) = if let Some(scope_genesis) = scope_genesis {
		let UnvalidatedSigned {
			sig: _,
			body: ScopeGenesisPayload {
				scope_id,
				owner,
				first_entry_hash,
			},
		} = scope_genesis.inner();

		if *scope_id != header.scope_id {
			return Err(Error::BadGenesisEntry);
		}

		if *owner != header.signer {
			return Err(Error::BadGenesisEntry);
		}

		if CurrentHash::digest(publish_version.canonical_bytes()) != *first_entry_hash {
			return Err(Error::BadGenesisEntry);
		}

		if header.prev_hash.0.iter().any(|b| *b != 0) {
			return Err(Error::BadGenesisEntry);
		}

		match db.begin_write_creating(&header.scope_id).await? {
			CreatingScopeLockResult::Ok {
				mut tx,
				global_size,
			} => {
				let entry = Entry {
					index: global_size,
					published_at: microsecond_timestamp(),
					payload: scope_genesis,
				};

				append_leaf(Global(&mut *tx), &entry).await?;
				tx.insert_scope_genesis(manifest.package.name.scope(), &entry)
					.await?;

				(tx.into_scope_transaction(), None)
			}
			CreatingScopeLockResult::AlreadyExists => {
				return Err(Error::BadGenesisEntry);
			}
		}
	} else {
		let result = db
			.begin_write_existing(
				&header.scope_id,
				&header.signer,
				PermissionWidth::Package(&package_version.pkg),
			)
			.await?;

		match result {
			ExistingScopeLockResult::Ok { tx, scope_size } => (tx, Some(scope_size)),
			ExistingScopeLockResult::Unauthorized => return Err(Error::Unauthorized),
			ExistingScopeLockResult::DoesntExist => return Err(Error::BadGenesisEntry),
		}
	};

	run_tx(tx, async move |tx| {
		let UnvalidatedSigned { sig, body } = publish_version.into_inner();

		let entry = Entry {
			index: scope_size.map_or(0, NonZero::get),
			published_at: microsecond_timestamp(),
			payload: ScopeEntryPayload::User(
				Signed::new(UnvalidatedSigned {
					sig,
					body: UserScopeOp::PublishVersion(body),
				})
				.expect(
					"body ops should always have the same canonical bytes as in their enum variant",
				),
			),
		};
		#[expect(clippy::match_wildcard_for_single_variants)]
		let body = match &entry.payload {
			ScopeEntryPayload::User(payload) => match &payload.inner().body {
				UserScopeOp::PublishVersion(body) => body,
				_ => unreachable!(),
			},
			_ => unreachable!(),
		};

		if !tx
			.insert_package_version(
				&body.header.scope_id,
				manifest.package.name.local_name(),
				entry.index,
				&body.package_version,
				&body.archive_hash,
				&body.manifest_hash,
			)
			.await?
		{
			return Err(Error::VersionAlreadyExists);
		}

		let mut scope_state = if let Some(scope_size) = scope_size {
			let tip = tx
				.scope_log_entry(body.header.scope_id, scope_size.get() - 1)
				.await?
				.ok_or(Error::InvalidPrevHash)?;

			if body.header.prev_hash != tip.hash() {
				return Err(Error::InvalidPrevHash);
			}

			tip.payload.header().post_state.clone()
		} else {
			ScopeState {
				owner: body.header.signer.clone(),
				scope_members_root: Default::default(),
				package_versions_root: Default::default(),
				package_deprecations_root: Default::default(),
			}
		};

		run_tree(
			&body.header.scope_id,
			tx,
			&mut scope_state,
			&body.header.post_state.package_versions_root,
			async |tree| {
				let result = tree
					.insert_new(
						body.package_version.clone(),
						PackageVersionValue {
							archive_hash: body.archive_hash,
							manifest_hash: body.manifest_hash,
							yank_state: None,
						},
					)
					.await;

				match result {
					Ok(_) => Ok(()),
					Err(_) => Err(Error::VersionAlreadyExists),
				}
			},
		)
		.await?;

		if body.header.post_state != scope_state {
			return Err(Error::InvalidScopeState);
		}

		append_leaf(Scoped(&body.header.scope_id, &mut *tx), &entry).await?;
		tx.insert_scope_entry(&entry).await?;

		let archive = blob.put_object(&body.archive_hash, std::io::Cursor::new(archive));
		let manifest =
			blob.put_object(&body.manifest_hash, std::io::Cursor::new(manifest_contents));
		try_join!(archive, manifest).map_err(Error::Internal)?;

		Ok(entry)
	})
	.await
}
