use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use futures::StreamExt as _;
use pesde::source::pesde::backend::DeprecateBody;
use pesde::source::pesde::backend::Entry;
use pesde::source::pesde::backend::EntryPayload;
use pesde::source::pesde::backend::EntrySeq;
use pesde::source::pesde::backend::IdentityId;
use pesde::source::pesde::backend::PublishBody;
use pesde::source::pesde::backend::ScopeEntry;
use pesde::source::pesde::backend::ScopeEntryBody;
use pesde::source::pesde::backend::ScopeEntryPayload;
use pesde::source::pesde::backend::ScopeManifest;
use pesde::source::pesde::backend::ScopeManifestUpdateBody;
use pesde::source::pesde::backend::ScopeMember;
use pesde::source::pesde::backend::ScopeSeq;
use pesde::source::pesde::backend::YankBody;
use sqlx::types::Uuid;

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
		Database::MySql(pool) => get_entry(pool, seq).await.map_err(Into::into),
	}
}

async fn get_scope_manifest(
	pool: &sqlx::MySqlPool,
	seq: EntrySeq,
) -> anyhow::Result<Option<ScopeManifest>> {
	let mut stream = sqlx::query!(
        r#"
        SELECT ScopeManifest.owner AS `owner: Uuid`, ScopeManifestMember.identity_id AS `identity_id: Uuid`, ScopeManifestMember.permissions
        FROM ScopeManifest
        LEFT JOIN ScopeManifestMember ON ScopeManifestMember.scope=ScopeManifest.scope AND ScopeManifestMember.seq=ScopeManifest.seq
        WHERE ScopeManifest.seq = ?
        "#,
        seq.0
    )
    .fetch(pool);

	let mut manifest = None;

	while let Some(row) = stream.next().await.transpose()? {
		let manifest = manifest.get_or_insert_with(|| ScopeManifest {
			owner: IdentityId(row.owner),
			members: Default::default(),
		});

		if let Some(identity_id) = row.identity_id {
			manifest.members.insert(
				IdentityId(identity_id),
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
	pool: &sqlx::MySqlPool,
	seq: EntrySeq,
) -> anyhow::Result<Option<ScopeEntry>> {
	let Some(scope_entry) = sqlx::query!(
        r#"
        SELECT sig, scope, prev_scope_entry_hash, scope_seq, prev_author_identity_seq, author_identity, kind
        FROM ScopeLogEntry
        WHERE seq = ?
        "#,
        seq.0
    )
    .fetch_optional(pool)
    .await? else {
		return Ok(None);
	};

	let payload = match &*scope_entry.kind {
		"publish" => {
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
		"yank" => {
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
		"deprecate" => {
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
		"manifest_update" => {
			let Some(manifest) = get_scope_manifest(pool, seq).await? else {
				return Ok(None);
			};
			ScopeEntryPayload::ManifestUpdate(ScopeManifestUpdateBody { manifest })
		}
		kind => panic!("invalid scope entry kind in database: {kind}"),
	};

	Ok(Some(ScopeEntry {
		sig: scope_entry.sig.parse()?,
		body: ScopeEntryBody {
			scope: scope_entry.scope.parse()?,
			prev_scope_entry_hash: scope_entry
				.prev_scope_entry_hash
				.map(|h| h.parse())
				.transpose()?,
			scope_seq: ScopeSeq(scope_entry.scope_seq),
			prev_author_identity_seq: scope_entry.prev_author_identity_seq.map(EntrySeq),
			author_identity: scope_entry.author_identity.try_into().map(IdentityId)?,
			payload,
		},
	}))
}

async fn get_entry(pool: &sqlx::MySqlPool, seq: EntrySeq) -> anyhow::Result<Option<Entry>> {
	let Some(entry) = sqlx::query!(
		r#"
        SELECT kind
        FROM LogEntry
        WHERE seq = ?
        "#,
		seq.0
	)
	.fetch_optional(pool)
	.await?
	else {
		return Ok(None);
	};

	Ok(Some(Entry {
		seq,
		payload: match &*entry.kind {
			"scope" => {
				let Some(scope_entry) = get_scope_entry(pool, seq).await? else {
					return Ok(None);
				};
				EntryPayload::Scope(scope_entry)
			}
			"register_identity" => todo!(),
			"identity_rotation" => todo!(),
			"admin_scope_transfer" => todo!(),
			kind => panic!("invalid entry kind in database: {kind}"),
		},
	}))
}
