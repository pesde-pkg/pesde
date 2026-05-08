use async_trait::async_trait;
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
use sqlx::SqlitePool;

#[async_trait]
pub trait LogRepo: Send + Sync + 'static {
	/// Gets the head of the log
	async fn head(&self) -> anyhow::Result<()>;

	/// Gets the consistency proof between two log sizes
	async fn consistency(&self) -> anyhow::Result<()>;

	/// Gets the inclusion proof for a given leaf index and tree size
	async fn inclusion(&self) -> anyhow::Result<()>;

	/// Gets the scope manifest by its sequence number
	async fn scope_manifest(&self, seq: EntrySeq) -> anyhow::Result<Option<ScopeManifest>>;

	/// Gets a scope-chained log entry by its sequence number
	async fn scope_entry(&self, seq: EntrySeq) -> anyhow::Result<Option<ScopeEntry>>;

	/// Gets a log entry by its sequence number
	async fn entry(&self, seq: EntrySeq) -> anyhow::Result<Option<Entry>>;
}

trait SqlStored {
	fn to_id(&self) -> u8;
	fn from_id(id: u8) -> Self;
}

impl SqlStored for EntryKind {
	fn to_id(&self) -> u8 {
		match self {
			Self::Scope => 1,
			Self::RegisterIdentity => 2,
			Self::IdentityRotation => 3,
			Self::AdminScopeTransfer => 4,
		}
	}

	fn from_id(id: u8) -> Self {
		match id {
			1 => Self::Scope,
			2 => Self::RegisterIdentity,
			3 => Self::IdentityRotation,
			4 => Self::AdminScopeTransfer,
			id => panic!("unrecognized id {id} for EntryKind"),
		}
	}
}

impl SqlStored for ScopeEntryKind {
	fn to_id(&self) -> u8 {
		match self {
			Self::Publish => 1,
			Self::Yank => 2,
			Self::Deprecate => 3,
			Self::ManifestUpdate => 4,
		}
	}

	fn from_id(id: u8) -> Self {
		match id {
			1 => Self::Publish,
			2 => Self::Yank,
			3 => Self::Deprecate,
			4 => Self::ManifestUpdate,
			id => panic!("unrecognized id {id} for ScopeEntryKind"),
		}
	}
}

pub struct SqliteLogRepo {
	pool: SqlitePool,
}

impl SqliteLogRepo {
	pub fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}
}

#[async_trait]
impl LogRepo for SqliteLogRepo {
	async fn head(&self) -> anyhow::Result<()> {
		todo!()
	}

	async fn consistency(&self) -> anyhow::Result<()> {
		todo!()
	}

	async fn inclusion(&self) -> anyhow::Result<()> {
		todo!()
	}

	async fn scope_manifest(&self, seq: EntrySeq) -> anyhow::Result<Option<ScopeManifest>> {
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
		.fetch(&self.pool);

		let mut manifest = None;

		while let Some(row) = stream.next().await.transpose()? {
			let manifest = manifest.get_or_insert_with(|| ScopeManifest {
				owner: row.owner.parse().as_ref().map(IdentityId::new).unwrap(),
				members: Default::default(),
			});

			if let Some(identity_id) = row.identity_id {
				manifest.members.insert(
					identity_id.parse().as_ref().map(IdentityId::new).unwrap(),
					ScopeMember {
						permissions: row
							.permissions
							.unwrap()
							.split(',')
							.map(|perm| perm.parse().unwrap())
							.collect(),
					},
				);
			}
		}

		Ok(manifest)
	}

	async fn scope_entry(&self, seq: EntrySeq) -> anyhow::Result<Option<ScopeEntry>> {
		let sqlite_seq = seq.0.cast_signed();

		let scope_entry = sqlx::query!(
			r#"
			SELECT sig, scope, prev_scope_entry_hash, scope_seq, prev_author_identity_seq, author_identity, kind as `kind: u8`
			FROM ScopeLogEntry
			WHERE seq = ?
			"#,
			sqlite_seq
		)
		.fetch_one(&self.pool)
		.await?;

		let scope_entry_kind = ScopeEntryKind::from_id(scope_entry.kind);
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
				.fetch_one(&self.pool)
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
				.fetch_one(&self.pool)
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
				.fetch_one(&self.pool)
				.await?;

				ScopeEntryPayload::Deprecate(DeprecateBody {
					name: deprecate_entry.name.parse()?,
					reason: deprecate_entry.reason.parse()?,
				})
			}
			ScopeEntryKind::ManifestUpdate => {
				ScopeEntryPayload::ManifestUpdate(ScopeManifestUpdateBody {
					manifest: self.scope_manifest(seq).await?.unwrap(),
				})
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
					.map(|seq| EntrySeq(seq.cast_unsigned())),
				author_identity: scope_entry.author_identity.parse()?,
				payload,
			},
		}))
	}

	async fn entry(&self, seq: EntrySeq) -> anyhow::Result<Option<Entry>> {
		let sqlite_seq = seq.0.cast_signed();

		let Some(entry) = sqlx::query!(
			r#"
			SELECT kind as `kind: u8`
			FROM LogEntry
			WHERE seq = ?
			"#,
			sqlite_seq
		)
		.fetch_optional(&self.pool)
		.await?
		else {
			return Ok(None);
		};

		let entry_kind = EntryKind::from_id(entry.kind);
		Ok(Some(Entry {
			seq,
			payload: match entry_kind {
				EntryKind::Scope => EntryPayload::Scope(self.scope_entry(seq).await?.unwrap()),
				EntryKind::RegisterIdentity => todo!(),
				EntryKind::IdentityRotation => todo!(),
				EntryKind::AdminScopeTransfer => todo!(),
			},
		}))
	}
}
