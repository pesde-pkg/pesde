use std::sync::Arc;

use futures::StreamExt as _;
use pesde::source::pesde::registry::*;
use sqlx::types::Uuid;

pub async fn mmr_size(pool: &sqlx::MySqlPool) -> anyhow::Result<u64> {
	Ok(sqlx::query!("SELECT size FROM Tree")
		.fetch_one(pool)
		.await?
		.size)
}

pub async fn get_hash(pool: &sqlx::MySqlPool, pos: u64) -> anyhow::Result<Option<Arc<[u8]>>> {
	Ok(sqlx::query!(
		"SELECT sha256 AS `sha256: Arc<[u8]>` FROM TreeNode WHERE pos = ?",
		pos
	)
	.fetch_optional(pool)
	.await?
	.map(|record| record.sha256))
}

pub async fn write_mmr(
	pool: &sqlx::MySqlPool,
) -> anyhow::Result<(sqlx::MySqlTransaction<'_>, u64)> {
	let mut tx = pool.begin().await?;
	let mmr_size = sqlx::query!("SELECT size FROM Tree FOR UPDATE")
		.fetch_one(&mut *tx)
		.await?
		.size;
	Ok((tx, mmr_size))
}

pub async fn append_hashes(
	tx: &mut sqlx::MySqlTransaction<'_>,
	pos: u64,
	elems: Vec<Arc<[u8]>>,
) -> anyhow::Result<()> {
	let mut query = sqlx::QueryBuilder::new(r"INSERT INTO TreeNode (pos, sha256) ");
	query.push_values((pos..).zip(elems), |mut b, (i, elem)| {
		b.push_bind(i);
		b.push_bind(elem);
	});
	query.build().execute(&mut **tx).await?;
	Ok(())
}

pub async fn get_scope_manifest(
	pool: &sqlx::MySqlPool,
	pos: u64,
) -> anyhow::Result<Option<ScopeManifest>> {
	let mut stream = sqlx::query!(
        r#"
        SELECT ScopeManifest.owner AS `owner: Uuid`, ScopeManifestMember.identity_id AS `identity_id: Uuid`, ScopeManifestMember.permissions
        FROM ScopeManifest
        LEFT JOIN ScopeManifestMember ON ScopeManifestMember.pos=ScopeManifest.pos
        WHERE ScopeManifest.pos = ?
        "#,
        pos
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
					permissions: ScopePermission::from_bits(row.permissions.unwrap()).unwrap(),
				},
			);
		}
	}

	Ok(manifest)
}

pub async fn get_scope_entry(
	pool: &sqlx::MySqlPool,
	pos: u64,
) -> anyhow::Result<Option<ScopeEntry>> {
	let Some(scope_entry) = sqlx::query!(
		r#"
        SELECT sig, scope, author_identity AS `author_identity: Uuid`, kind
        FROM ScopeLogEntry
        WHERE pos = ?
        "#,
		pos
	)
	.fetch_optional(pool)
	.await?
	else {
		return Ok(None);
	};

	let payload = match &*scope_entry.kind {
		"publish" => {
			let publish_entry = sqlx::query!(
				r#"
                SELECT name, version, archive_hash
                FROM PublishScopeLogEntry
                WHERE pos = ?
                "#,
				pos
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
                SELECT PublishScopeLogEntry.name, PublishScopeLogEntry.version
                FROM YankScopeLogEntry
				INNER JOIN PublishScopeLogEntry ON PublishScopeLogEntry.pos=YankScopeLogEntry.publish_pos
                WHERE YankScopeLogEntry.pos = ?
                "#,
				pos
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
                WHERE pos = ?
                "#,
				pos
			)
			.fetch_one(pool)
			.await?;

			ScopeEntryPayload::Deprecate(DeprecateBody {
				name: deprecate_entry.name.parse()?,
				reason: deprecate_entry.reason,
			})
		}
		"manifest_update" => {
			let Some(manifest) = get_scope_manifest(pool, pos).await? else {
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
			author_identity: IdentityId(scope_entry.author_identity),
			payload,
		},
	}))
}

pub async fn get_entry(pool: &sqlx::MySqlPool, pos: u64) -> anyhow::Result<Option<Entry>> {
	let Some(entry) = sqlx::query!(
		r#"
        SELECT kind
        FROM LogEntry
        WHERE pos = ?
        "#,
		pos
	)
	.fetch_optional(pool)
	.await?
	else {
		return Ok(None);
	};

	Ok(Some(Entry {
		pos,
		payload: match &*entry.kind {
			"scope" => {
				let Some(scope_entry) = get_scope_entry(pool, pos).await? else {
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
