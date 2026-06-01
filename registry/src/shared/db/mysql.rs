use futures::StreamExt as _;
use pesde::hash::Hash;
use pesde::source::pesde::registry::*;
use sqlx::types::Uuid;

pub async fn mmr_size(pool: &sqlx::MySqlPool) -> anyhow::Result<u64> {
	Ok(
		sqlx::query!("SELECT COUNT(*) as `mmr_size: u64` FROM TreeNode")
			.fetch_one(pool)
			.await?
			.mmr_size,
	)
}

pub async fn get_hash(pool: &sqlx::MySqlPool, pos: u64) -> anyhow::Result<Option<Hash>> {
	Ok(
		sqlx::query!("SELECT sha256 FROM TreeNode WHERE pos = ?", pos)
			.fetch_optional(pool)
			.await?
			.map(|record| Hash::new(pesde::hash::HashAlgorithm::Sha256, record.sha256).unwrap()),
	)
}

pub async fn write_mmr(
	pool: &sqlx::MySqlPool,
) -> anyhow::Result<(sqlx::MySqlTransaction<'_>, u64)> {
	let mut tx = pool.begin().await?;
	let mmr_size = sqlx::query!("SELECT COUNT(*) as `mmr_size: u64` FROM TreeNode FOR UPDATE")
		.fetch_one(&mut *tx)
		.await?
		.mmr_size;
	Ok((tx, mmr_size))
}

pub async fn append_hashes(
	tx: &mut sqlx::MySqlTransaction<'_>,
	pos: u64,
	elems: Vec<Hash>,
) -> anyhow::Result<()> {
	let mut query = sqlx::QueryBuilder::new(r"INSERT INTO TreeNode (pos, sha256) ");
	query.push_values((pos..).zip(elems), |mut b, (i, elem)| {
		b.push_bind(i);
		match elem.algorithm() {
			pesde::hash::HashAlgorithm::Sha256 => b.push_bind(elem.hash()),
			algorithm => panic!("unsupported hash algorithm in MMR store: {algorithm}"),
		};
	});
	query.build().execute(&mut **tx).await?;
	Ok(())
}

pub async fn get_pos(pool: &sqlx::MySqlPool, seq: EntrySeq) -> anyhow::Result<Option<u64>> {
	Ok(
		sqlx::query!("SELECT pos FROM LogEntry WHERE seq = ?", seq.0)
			.fetch_optional(pool)
			.await?
			.map(|record| record.pos),
	)
}

pub async fn get_scope_manifest(
	pool: &sqlx::MySqlPool,
	seq: EntrySeq,
) -> anyhow::Result<Option<ScopeManifest>> {
	let mut stream = sqlx::query!(
        r#"
        SELECT ScopeManifest.owner AS `owner: Uuid`, ScopeManifestMember.identity_id AS `identity_id: Uuid`, ScopeManifestMember.permissions
        FROM ScopeManifest
        LEFT JOIN ScopeManifestMember ON ScopeManifestMember.seq=ScopeManifest.seq
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
					permissions: ScopePermission::from_bits(row.permissions.unwrap()).unwrap(),
				},
			);
		}
	}

	Ok(manifest)
}

pub async fn get_scope_entry(
	pool: &sqlx::MySqlPool,
	seq: EntrySeq,
) -> anyhow::Result<Option<ScopeEntry>> {
	let Some(scope_entry) = sqlx::query!(
		r#"
        SELECT sig, scope, scope_seq, author_identity AS `author_identity: Uuid`, kind
        FROM ScopeLogEntry
        WHERE seq = ?
        "#,
		seq.0
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
                WHERE seq = ?
                "#,
				seq.0
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
                WHERE seq = ?
                "#,
				seq.0
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
                WHERE seq = ?
                "#,
				seq.0
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
			scope_seq: ScopeSeq(scope_entry.scope_seq),
			author_identity: IdentityId(scope_entry.author_identity),
			payload,
		},
	}))
}

pub async fn get_entry(pool: &sqlx::MySqlPool, seq: EntrySeq) -> anyhow::Result<Option<Entry>> {
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
