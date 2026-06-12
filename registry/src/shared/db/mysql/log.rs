use anyhow::Context as _;
use async_trait::async_trait;
use futures::StreamExt as _;
use pesde::names::Scope;
use pesde::signature::KeyKind;
use pesde::signature::PublicKey;
use pesde::signature::Signature;
use pesde::source::pesde::registry::*;
use sqlx::MySqlPool;

use crate::features::log::Repository;
use crate::shared::db::EntryKind;
use crate::shared::db::mysql::MySqlBackend;
use crate::shared::db::mysql::ScopeEntryKind;
use crate::shared::db::mysql::build_public_key;
use crate::shared::db::mysql::build_publish_body;

#[async_trait]
impl Repository for MySqlBackend {
	async fn entry(&self, pos: u64) -> anyhow::Result<Option<Entry<EntryPayload>>> {
		let Some(rec) = sqlx::query!(
			r#"
			SELECT kind AS `kind: EntryKind`
			FROM LogEntry
			WHERE pos = ?
			"#,
			pos,
		)
		.fetch_optional(&self.pool)
		.await?
		else {
			return Ok(None);
		};

		match rec.kind {
			EntryKind::Scope => {
				let Some(env) = sqlx::query!(
					r#"
                    SELECT ScopeLogEntry.sig, Scope.scope, ScopeLogEntry.author_identity AS `author_identity: IdentityId`, ScopeLogEntry.kind AS `kind: ScopeEntryKind`
                    FROM ScopeLogEntry
                    INNER JOIN Scope ON Scope.id=ScopeLogEntry.scope_id
                    WHERE ScopeLogEntry.pos = ?
                    "#,
					pos,
				)
				.fetch_optional(&self.pool)
				.await?
				else {
					return Ok(None);
				};

				let sig = env.sig.parse()?;
				let scope = env.scope.parse()?;
				let author_identity = env.author_identity;

				let scope_entry = match env.kind {
					ScopeEntryKind::Publish => {
						let publish = sqlx::query!(
							r#"
                            SELECT Package.name, PublishScopeLogEntry.version, PublishScopeLogEntry.archive_hash, PublishScopeLogEntry.description, PublishScopeLogEntry.license, PublishScopeLogEntry.repository
                            FROM PublishScopeLogEntry
                            INNER JOIN Package ON Package.genesis_pos=PublishScopeLogEntry.package_pos
                            WHERE PublishScopeLogEntry.pos = ?
                            "#,
							pos,
						)
						.fetch_one(&self.pool)
						.await?;

						ScopeEntry::Publish(SignedEntry::new(
							sig,
							ScopeEntryBody {
								scope,
								author_identity,
								payload: build_publish_body(
									&self.pool,
									pos,
									&publish.name,
									&publish.version,
									&publish.archive_hash,
									publish.description,
									publish.license,
									&publish.repository,
								)
								.await?,
							},
						))
					}
					ScopeEntryKind::Yank => {
						let row = sqlx::query!(
							r#"
                            SELECT Package.name, PublishScopeLogEntry.version, YankScopeLogEntry.action AS `action: YankRetraction`
                            FROM YankScopeLogEntry
                            INNER JOIN PublishScopeLogEntry ON PublishScopeLogEntry.pos=YankScopeLogEntry.publish_pos
                            INNER JOIN Package ON Package.genesis_pos=PublishScopeLogEntry.package_pos
                            WHERE YankScopeLogEntry.pos = ?
                            "#,
							pos,
						)
						.fetch_one(&self.pool)
						.await?;

						ScopeEntry::Yank(SignedEntry::new(
							sig,
							ScopeEntryBody {
								scope,
								author_identity,
								payload: YankBody {
									name: row.name.parse()?,
									version: row.version.parse()?,
									action: row.action,
								},
							},
						))
					}
					ScopeEntryKind::Deprecate => {
						let row = sqlx::query!(
							r#"
                            SELECT Package.name, DeprecateScopeLogEntry.reason
                            FROM DeprecateScopeLogEntry
                            INNER JOIN Package ON Package.genesis_pos=DeprecateScopeLogEntry.package_pos
                            WHERE DeprecateScopeLogEntry.pos = ?
                            "#,
							pos,
						)
						.fetch_one(&self.pool)
						.await?;

						ScopeEntry::Deprecate(SignedEntry::new(
							sig,
							ScopeEntryBody {
								scope,
								author_identity,
								payload: DeprecateBody {
									name: row.name.parse()?,
									reason: row.reason.parse()?,
								},
							},
						))
					}
					ScopeEntryKind::ManifestUpdate => {
						let (_, manifest) = read_scope_manifest(&self.pool, pos).await?;

						ScopeEntry::ManifestUpdate(SignedEntry::new(
							sig,
							ScopeEntryBody {
								scope,
								author_identity,
								payload: ScopeManifestUpdateBody { manifest },
							},
						))
					}
				};

				Ok(Some(Entry {
					pos,
					payload: EntryPayload::Scope(scope_entry),
				}))
			}
			EntryKind::RegisterIdentity => {
				let Some((sig, _, identity_id, public_key)) =
					read_key_entry(&self.pool, pos).await?
				else {
					return Ok(None);
				};

				Ok(Some(Entry {
					pos,
					payload: EntryPayload::Identity(IdentityEntry::Register(SignedEntry::new(
						sig,
						RegisterIdentityBody {
							identity_id,
							public_key,
						},
					))),
				}))
			}
			EntryKind::IdentityRotation => {
				let Some((new_sig, old_sig, identity_id, new_public_key)) =
					read_key_entry(&self.pool, pos).await?
				else {
					return Ok(None);
				};

				Ok(Some(Entry {
					pos,
					payload: EntryPayload::Identity(IdentityEntry::Rotation(
						IdentityRotationEntry::new(
							old_sig
								.context("rotation entry is missing its authorising signature")?,
							new_sig,
							IdentityRotationBody {
								identity_id,
								new_public_key,
							},
						),
					)),
				}))
			}
			EntryKind::AdminScopeTransfer => {
				let (scope, manifest) = read_scope_manifest(&self.pool, pos).await?;
				Ok(Some(Entry {
					pos,
					payload: EntryPayload::AdminScopeTransfer(AdminScopeTransfer {
						scope,
						manifest,
					}),
				}))
			}
		}
	}
}

async fn read_key_entry(
	pool: &MySqlPool,
	pos: u64,
) -> anyhow::Result<Option<(Signature, Option<Signature>, IdentityId, PublicKey)>> {
	let Some(row) = sqlx::query!(
		r#"
		SELECT sig, authorising_sig, identity_id AS `identity_id: IdentityId`, algorithm AS `algorithm: KeyKind`, public_key
		FROM IdentityKeyEntry
		WHERE pos = ?
		"#,
		pos,
	)
	.fetch_optional(pool)
	.await?
	else {
		return Ok(None);
	};

	Ok(Some((
		row.sig.parse()?,
		row.authorising_sig.as_deref().map(str::parse).transpose()?,
		row.identity_id,
		build_public_key(row.algorithm, row.public_key)?,
	)))
}

async fn read_scope_manifest(pool: &MySqlPool, pos: u64) -> anyhow::Result<(Scope, ScopeManifest)> {
	let mut members = sqlx::query!(
		r#"
		SELECT Scope.scope, ScopeManifest.owner AS `owner: IdentityId`, ScopeManifestMember.identity_id AS `identity_id: IdentityId`, ScopeManifestMember.package
		FROM ScopeManifest
		INNER JOIN Scope ON Scope.id=ScopeManifest.scope_id
		LEFT JOIN ScopeManifestMember ON ScopeManifestMember.pos=ScopeManifest.pos
		WHERE ScopeManifest.pos = ?
		"#,
		pos,
	)
	.fetch(pool);

	let mut result = None;
	while let Some(row) = members.next().await.transpose()? {
		let manifest = if let Some((_, manifest)) = &mut result {
			manifest
		} else {
			let (_, manifest) = result.insert((
				row.scope.parse()?,
				ScopeManifest {
					owner: row.owner,
					members: Default::default(),
				},
			));

			manifest
		};

		if let Some(identity_id) = row.identity_id {
			use std::collections::btree_map::Entry;

			let entry = manifest.members.entry(identity_id);
			let package = row.package.unwrap();

			if package.is_empty() {
				match entry {
					Entry::Vacant(entry) => entry.insert(ScopeMember::AllPackages),
					Entry::Occupied(_) => {
						anyhow::bail!("wildcard member with other packages")
					}
				};
			} else {
				let member = entry.or_insert_with(|| ScopeMember::Packages(Default::default()));
				match member {
					ScopeMember::AllPackages => {
						anyhow::bail!("per-package member with all packages")
					}
					ScopeMember::Packages(packages) => {
						packages.insert(package.parse()?);
					}
				}
			}
		}
	}

	result.context("no matching scope manifest found")
}
