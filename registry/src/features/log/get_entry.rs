use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use futures::StreamExt as _;
use pesde::source::pesde::backend::DeprecateBody;
use pesde::source::pesde::backend::Entry;
use pesde::source::pesde::backend::EntryKind;
use pesde::source::pesde::backend::EntryPayload;
use pesde::source::pesde::backend::EntrySeq;
use pesde::source::pesde::backend::IdentityId;
use pesde::source::pesde::backend::PublishBody;
use pesde::source::pesde::backend::ScopeEntry;
use pesde::source::pesde::backend::ScopeEntryBody;
use pesde::source::pesde::backend::ScopeEntryKind;
use pesde::source::pesde::backend::ScopeEntryPayload;
use pesde::source::pesde::backend::ScopeManifest;
use pesde::source::pesde::backend::ScopeManifestUpdateBody;
use pesde::source::pesde::backend::ScopeMember;
use pesde::source::pesde::backend::ScopeSeq;
use pesde::source::pesde::backend::YankBody;

use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;

#[get("/v2/log/entry/{seq}")]
pub async fn http(app_state: web::Data<AppState>, seq: web::Path<EntrySeq>) -> ControllerResult {
	let Some(entry) = handler(&app_state.database, seq.into_inner()).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};
	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(db: &Database, seq: EntrySeq) -> AppResult<Option<Entry>> {
	match db {
		Database::Sqlite(pool) => get_entry(pool, seq).await.map_err(Into::into),
	}
}

fn entry_kind_from_id(id: u8) -> EntryKind {
	match id {
		1 => EntryKind::Scope,
		2 => EntryKind::RegisterIdentity,
		3 => EntryKind::IdentityRotation,
		4 => EntryKind::AdminScopeTransfer,
		_ => panic!("unrecognized id {id} for EntryKind"),
	}
}

fn scope_entry_kind_from_id(id: u8) -> ScopeEntryKind {
	match id {
		1 => ScopeEntryKind::Publish,
		2 => ScopeEntryKind::Yank,
		3 => ScopeEntryKind::Deprecate,
		4 => ScopeEntryKind::ManifestUpdate,
		_ => panic!("unrecognized id {id} for ScopeEntryKind"),
	}
}

async fn get_scope_manifest(
	pool: &sqlx::SqlitePool,
	seq: EntrySeq,
) -> anyhow::Result<Option<ScopeManifest>> {
	let sqlite_seq = seq.0.cast_signed();

	let mut stream = sqlx::query!(
        r#"
        SELECT ScopeManifest.owner, ScopeManifestMember.identity_id, ScopeManifestMember.permissions
        FROM ScopeManifest
        LEFT JOIN ScopeManifestMember ON ScopeManifestMember.scope=ScopeManifest.scope AND ScopeManifestMember.seq=ScopeManifest.seq
        WHERE ScopeManifest.seq = ?
        "#,
        sqlite_seq
    )
    .fetch(pool);

	let mut manifest = None;

	while let Some(row) = stream.next().await.transpose()? {
		let manifest = match &mut manifest {
			Some(m) => m,
			None => manifest.insert(ScopeManifest {
				owner: row.owner.parse().map(|o| IdentityId::new(&o))?,
				members: Default::default(),
			}),
		};

		if let Some(identity_id) = row.identity_id {
			manifest.members.insert(
				identity_id.parse()?,
				ScopeMember {
					permissions: row
						.permissions
						.unwrap()
						.split(',')
						.map(str::parse)
						.collect::<Result<_, _>>()?,
				},
			);
		}
	}

	Ok(manifest)
}

async fn get_scope_entry(
	pool: &sqlx::SqlitePool,
	seq: EntrySeq,
) -> anyhow::Result<Option<ScopeEntry>> {
	let sqlite_seq = seq.0.cast_signed();

	let Some(scope_entry) = sqlx::query!(
        r#"
        SELECT sig, scope, prev_scope_entry_hash, scope_seq, prev_author_identity_seq, author_identity, kind as `kind: u8`
        FROM ScopeLogEntry
        WHERE seq = ?
        "#,
        sqlite_seq
    )
    .fetch_optional(pool)
    .await? else {
		return Ok(None);
	};

	let scope_entry_kind = scope_entry_kind_from_id(scope_entry.kind);
	let payload = match scope_entry_kind {
		ScopeEntryKind::Publish => {
			let publish_entry = sqlx::query!(
				r#"
                SELECT name, version, archive_hash
                FROM PublishScopeLogEntry
                WHERE scope = ? AND scope_seq = ?
                "#,
				scope_entry.scope,
				scope_entry.scope_seq
			)
			.fetch_one(pool)
			.await?;

			ScopeEntryPayload::Publish(PublishBody {
				name: publish_entry.name.parse()?,
				version: publish_entry.version.parse()?,
				archive_hash: publish_entry.archive_hash.parse()?,
			})
		}
		ScopeEntryKind::Yank => {
			let yank_entry = sqlx::query!(
				r#"
                SELECT name, version
                FROM YankScopeLogEntry
                WHERE scope = ? AND scope_seq = ?
                "#,
				scope_entry.scope,
				scope_entry.scope_seq
			)
			.fetch_one(pool)
			.await?;

			ScopeEntryPayload::Yank(YankBody {
				name: yank_entry.name.parse()?,
				version: yank_entry.version.parse()?,
			})
		}
		ScopeEntryKind::Deprecate => {
			let deprecate_entry = sqlx::query!(
				r#"
                SELECT name, reason
                FROM DeprecateScopeLogEntry
                WHERE scope = ? AND scope_seq = ?
                "#,
				scope_entry.scope,
				scope_entry.scope_seq
			)
			.fetch_one(pool)
			.await?;

			ScopeEntryPayload::Deprecate(DeprecateBody {
				name: deprecate_entry.name.parse()?,
				reason: deprecate_entry.reason,
			})
		}
		ScopeEntryKind::ManifestUpdate => {
			let Some(manifest) = get_scope_manifest(pool, seq).await? else {
				return Ok(None);
			};
			ScopeEntryPayload::ManifestUpdate(ScopeManifestUpdateBody { manifest })
		}
	};

	Ok(Some(ScopeEntry {
		sig: scope_entry.sig.parse()?,
		body: ScopeEntryBody {
			scope: scope_entry.scope.parse()?,
			prev_scope_entry_hash: scope_entry
				.prev_scope_entry_hash
				.map(|h| h.parse())
				.transpose()?,
			scope_seq: ScopeSeq(scope_entry.scope_seq.cast_unsigned()),
			prev_author_identity_seq: scope_entry
				.prev_author_identity_seq
				.map(|s| EntrySeq(s.cast_unsigned())),
			author_identity: scope_entry.author_identity.parse()?,
			payload,
		},
	}))
}

async fn get_entry(pool: &sqlx::SqlitePool, seq: EntrySeq) -> anyhow::Result<Option<Entry>> {
	let sqlite_seq = seq.0.cast_signed();

	let Some(entry) = sqlx::query!(
		r#"
        SELECT kind as `kind: u8`
        FROM LogEntry
        WHERE seq = ?
        "#,
		sqlite_seq
	)
	.fetch_optional(pool)
	.await?
	else {
		return Ok(None);
	};

	Ok(Some(Entry {
		seq,
		payload: match entry_kind_from_id(entry.kind) {
			EntryKind::Scope => {
				let Some(scope_entry) = get_scope_entry(pool, seq).await? else {
					return Ok(None);
				};
				EntryPayload::Scope(scope_entry)
			}
			EntryKind::RegisterIdentity => todo!(),
			EntryKind::IdentityRotation => todo!(),
			EntryKind::AdminScopeTransfer => todo!(),
		},
	}))
}
