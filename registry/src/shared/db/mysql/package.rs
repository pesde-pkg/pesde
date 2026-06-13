use std::num::NonZero;

use async_trait::async_trait;
use futures::TryStreamExt as _;
use jiff::Timestamp;
use pesde::bounded::Bounded;
use pesde::names::PackageName;
use pesde::signature::Signature;
use pesde::source::pesde::registry::*;
use semver::Version;
use sqlx::MySqlPool;
use sqlx::error::DatabaseError;

use crate::features::package::Repository;
use crate::shared::db::PackageWriteError;
use crate::shared::db::WriteStore;
use crate::shared::db::mysql::DependencyKind;
use crate::shared::db::mysql::MySqlBackend;
use crate::shared::db::mysql::ScopeEntryKind;
use crate::shared::db::mysql::as_tx;
use crate::shared::db::mysql::build_publish_body;
use crate::shared::db::mysql::insert_chunked;
use crate::shared::db::mysql::insert_scope_envelope;
use crate::util::semver_ord;

#[async_trait]
impl Repository for MySqlBackend {
	async fn package_version(
		&self,
		name: &PackageName,
		version: &Version,
	) -> anyhow::Result<Option<PackageVersionResponse>> {
		let Some(row) = sqlx::query!(
			r#"
            SELECT
					UNIX_TIMESTAMP(LogEntry.published_at) AS `published_at!`,
					ScopeLogEntry.pos, ScopeLogEntry.sig, ScopeLogEntry.author_identity AS `author_identity: IdentityId`,
                	Package.name, Package.genesis_pos,
					PublishScopeLogEntry.version, PublishScopeLogEntry.archive_hash, PublishScopeLogEntry.description, PublishScopeLogEntry.license, PublishScopeLogEntry.repository
            FROM ScopeLogEntry
			INNER JOIN LogEntry ON LogEntry.pos=ScopeLogEntry.pos
            INNER JOIN PublishScopeLogEntry ON PublishScopeLogEntry.pos=ScopeLogEntry.pos
            INNER JOIN Package ON Package.genesis_pos=PublishScopeLogEntry.package_pos
            INNER JOIN Scope ON Scope.id=Package.scope_id
            WHERE Scope.scope = ? AND Package.name = ? AND PublishScopeLogEntry.version = ?
            "#,
			name.scope().as_str(),
			name.name().as_str(),
			version.to_string(),
		)
		.fetch_optional(&self.pool)
		.await?
		else {
			return Ok(None);
		};

		let publish = build_publish_body(
			&self.pool,
			row.pos,
			&row.name,
			&row.version,
			&row.archive_hash,
			row.description,
			row.license,
			&row.repository,
		)
		.await?;

		Ok(Some(PackageVersionResponse {
			publish: Entry {
				pos: row.pos,
				published_at: Timestamp::from_second(row.published_at)?,
				payload: SignedEntry::new(
					row.sig.parse()?,
					ScopeEntryBody {
						scope: name.scope().clone(),
						author_identity: row.author_identity,
						payload: publish,
					},
				),
			},
			yank: current_yank(&self.pool, row.pos, name, version).await?,
		}))
	}

	async fn package_info(
		&self,
		name: &PackageName,
	) -> anyhow::Result<Option<PackageInfoResponse>> {
		let Some(package) = sqlx::query!(
			r#"
			SELECT Package.genesis_pos, PublishScopeLogEntry.version
			FROM Package
			INNER JOIN Scope ON Scope.id=Package.scope_id
			INNER JOIN PublishScopeLogEntry	ON PublishScopeLogEntry.package_pos=Package.genesis_pos
			LEFT JOIN YankScopeLogEntry ON YankScopeLogEntry.publish_pos=PublishScopeLogEntry.pos AND YankScopeLogEntry.pos=(SELECT pos FROM YankScopeLogEntry WHERE publish_pos=PublishScopeLogEntry.pos ORDER BY pos DESC LIMIT 1)
			WHERE Scope.scope = ? AND Package.name = ?
			ORDER BY (YankScopeLogEntry.action IS NULL OR YankScopeLogEntry.action = 'revoke') DESC, PublishScopeLogEntry.version_ord DESC
			LIMIT 1
			"#,
			name.scope().as_str(),
			name.name().as_str(),
		)
		.fetch_optional(&self.pool)
		.await?
		else {
			return Ok(None);
		};

		Ok(Some(PackageInfoResponse {
			deprecation: current_deprecation(&self.pool, package.genesis_pos, name).await?,
			latest_version: package.version.parse()?,
		}))
	}

	async fn package_versions(
		&self,
		name: &PackageName,
		after: u64,
		limit: NonZero<u8>,
	) -> anyhow::Result<PackageVersionsResponse> {
		let mut rows = sqlx::query!(
			r#"
			SELECT COUNT(PublishScopeLogEntry.pos) AS `total: u64`, PublishScopeLogEntry.version
			FROM Package
			INNER JOIN Scope ON Scope.id=Package.scope_id
			INNER JOIN PublishScopeLogEntry ON PublishScopeLogEntry.package_pos=Package.genesis_pos
			WHERE Scope.scope = ? AND Package.name = ? AND PublishScopeLogEntry.pos > ?
			ORDER BY PublishScopeLogEntry.pos DESC
			LIMIT ?
			"#,
			name.scope().as_str(),
			name.name().as_str(),
			after,
			limit,
		)
		.fetch(&self.pool);

		let mut response = PackageVersionsResponse {
			versions: vec![],
			total: 0,
		};

		while let Some(row) = rows.try_next().await? {
			response.total = row.total;
			let Some(version) = row.version else {
				continue;
			};
			// TODO: look at inlining this into 1 query
			response.versions.push(
				self.package_version(name, &version.parse()?)
					.await?
					.unwrap(),
			);
		}

		Ok(response)
	}

	async fn insert_publish(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &ScopeEntryBody<PublishBody>,
	) -> Result<(), PackageWriteError> {
		let publish = &body.payload;
		let tx = as_tx(tx);

		let scope_id = insert_scope_envelope(tx, pos, sig, body, ScopeEntryKind::Publish).await?;

		let package_pos = if let Some(row) = sqlx::query!(
			"SELECT genesis_pos FROM Package WHERE scope_id = ? AND name = ?",
			scope_id,
			publish.name.as_str(),
		)
		.fetch_optional(&mut **tx)
		.await
		.map_err(anyhow::Error::from)?
		{
			row.genesis_pos
		} else {
			sqlx::query!(
				"INSERT INTO Package (genesis_pos, scope_id, name) VALUES (?, ?, ?)",
				pos,
				scope_id,
				publish.name.as_str(),
			)
			.execute(&mut **tx)
			.await
			.map_err(anyhow::Error::from)?;
			pos
		};

		match sqlx::query!(
			"INSERT INTO PublishScopeLogEntry (pos, package_pos, version, version_ord, archive_hash, description, license, repository) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
			pos,
			package_pos,
			publish.version.to_string(),
			semver_ord(&publish.version),
			publish.archive_hash.to_string(),
			publish.description.as_str(),
			publish.license.as_str(),
			publish.repository.as_ref().map(ToString::to_string),
		)
		.execute(&mut **tx)
		.await
		{
			Ok(_) => {}
			Err(e)
				if e.as_database_error()
					.is_some_and(DatabaseError::is_unique_violation) =>
			{
				return Err(PackageWriteError::VersionAlreadyExists);
			}
			Err(e) => return Err(anyhow::Error::from(e).into()),
		}

		insert_chunked!(
			tx,
			"PublishAuthor",
			["pos", "seq", "author"],
			(0u8..).zip(publish.authors.iter()),
			|mut b, (seq, author)| {
				b.push_bind(pos).push_bind(seq).push_bind(author.as_str());
			},
		)
		.await
		.map_err(anyhow::Error::from)?;

		insert_chunked!(
			tx,
			"PublishDependency",
			[
				"pos",
				"alias",
				"dependency_type",
				"kind",
				"name",
				"version_req",
				"registry",
				"realm"
			],
			publish.dependencies.iter(),
			|mut b, (alias, (specifier, ty))| {
				let (kind, name, version_req, registry, realm) = match specifier {
					RegistryDependencySpecifier::Pesde(spec) => (
						DependencyKind::Pesde,
						spec.name.to_string(),
						spec.version.to_string(),
						spec.registry.as_ref().map(ToString::to_string),
						spec.realm,
					),
					RegistryDependencySpecifier::Wally(spec) => (
						DependencyKind::Wally,
						spec.name.to_string(),
						spec.version.to_string(),
						Some(spec.index.to_string()),
						Some(spec.realm),
					),
				};

				b.push_bind(pos)
					.push_bind(alias.as_str())
					.push_bind(ty)
					.push_bind(kind)
					.push_bind(name)
					.push_bind(version_req)
					.push_bind(registry)
					.push_bind(realm);
			},
		)
		.await
		.map_err(anyhow::Error::from)?;

		Ok(())
	}

	async fn insert_yank(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &ScopeEntryBody<YankBody>,
	) -> Result<(), PackageWriteError> {
		let conn = as_tx(tx);

		let Some(current_state) = sqlx::query!(
			r#"
            SELECT PublishScopeLogEntry.pos, YankScopeLogEntry.action AS `action: YankRetraction`
			FROM PublishScopeLogEntry
			LEFT JOIN YankScopeLogEntry ON YankScopeLogEntry.publish_pos=PublishScopeLogEntry.pos
            INNER JOIN Package ON Package.genesis_pos=PublishScopeLogEntry.package_pos
            INNER JOIN Scope ON Scope.id=Package.scope_id
            WHERE Scope.scope = ? AND Package.name = ? AND PublishScopeLogEntry.version = ?
			ORDER BY YankScopeLogEntry.pos DESC
			LIMIT 1
            "#,
			body.scope.as_str(),
			body.payload.name.as_str(),
			body.payload.version.to_string(),
		)
		.fetch_optional(&mut **conn)
		.await
		.map_err(anyhow::Error::from)?
		else {
			return Err(PackageWriteError::UnknownPackageVersion);
		};

		match (current_state.action, body.payload.action) {
			(None | Some(YankRetraction::Revoke), YankRetraction::Add)
			| (Some(YankRetraction::Add), YankRetraction::Revoke) => {}
			(Some(YankRetraction::Add), YankRetraction::Add) => {
				return Err(PackageWriteError::AlreadyYanked);
			}
			(None | Some(YankRetraction::Revoke), YankRetraction::Revoke) => {
				return Err(PackageWriteError::NotYanked);
			}
		}

		insert_scope_envelope(conn, pos, sig, body, ScopeEntryKind::Yank).await?;

		sqlx::query!(
			"INSERT INTO YankScopeLogEntry (pos, publish_pos, action) VALUES (?, ?, ?)",
			pos,
			current_state.pos,
			body.payload.action,
		)
		.execute(&mut **conn)
		.await
		.map_err(anyhow::Error::from)?;

		Ok(())
	}

	async fn insert_deprecate(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &ScopeEntryBody<DeprecateBody>,
	) -> Result<(), PackageWriteError> {
		let conn = as_tx(tx);

		let Some(current_state) = sqlx::query!(
			r#"
            SELECT Package.genesis_pos, DeprecateScopeLogEntry.reason
			FROM Package
            INNER JOIN Scope ON Scope.id=Package.scope_id
			LEFT JOIN DeprecateScopeLogEntry ON DeprecateScopeLogEntry.package_pos=Package.genesis_pos
            WHERE Scope.scope = ? AND Package.name = ?
			ORDER BY DeprecateScopeLogEntry.pos DESC
			LIMIT 1
            "#,
			body.scope.as_str(),
			body.payload.name.as_str(),
		)
		.fetch_optional(&mut **conn)
		.await
		.map_err(anyhow::Error::from)?
		else {
			return Err(PackageWriteError::UnknownPackageVersion);
		};

		match (
			current_state.reason.as_deref().map(str::is_empty),
			body.payload.reason.is_empty(),
		) {
			(None | Some(true), false) | (Some(false), true) => {}
			(None | Some(true), true) => return Err(PackageWriteError::NotDeprecated),
			(Some(false), false) => return Err(PackageWriteError::AlreadyDeprecated),
		}

		insert_scope_envelope(conn, pos, sig, body, ScopeEntryKind::Deprecate).await?;

		sqlx::query!(
			"INSERT INTO DeprecateScopeLogEntry (pos, package_pos, reason) VALUES (?, ?, ?)",
			pos,
			current_state.genesis_pos,
			body.payload.reason.as_str(),
		)
		.execute(&mut **conn)
		.await
		.map_err(anyhow::Error::from)?;

		Ok(())
	}
}

// TODO: inline these into 1 query

async fn current_yank(
	pool: &MySqlPool,
	publish_pos: u64,
	name: &PackageName,
	version: &Version,
) -> anyhow::Result<Option<Entry<YankScopeEntry>>> {
	let Some(row) = sqlx::query!(
		r#"
		SELECT UNIX_TIMESTAMP(LogEntry.published_at) AS `published_at!`, YankScopeLogEntry.pos, ScopeLogEntry.sig, ScopeLogEntry.author_identity AS `author_identity: IdentityId`, YankScopeLogEntry.action AS `action: YankRetraction`
		FROM YankScopeLogEntry
		INNER JOIN ScopeLogEntry ON ScopeLogEntry.pos=YankScopeLogEntry.pos
		INNER JOIN LogEntry ON LogEntry.pos=ScopeLogEntry.pos
		WHERE YankScopeLogEntry.publish_pos = ?
		ORDER BY YankScopeLogEntry.pos DESC
		LIMIT 1
		"#,
		publish_pos,
	)
	.fetch_optional(pool)
	.await?
	else {
		return Ok(None);
	};

	if row.action != YankRetraction::Add {
		return Ok(None);
	}

	Ok(Some(Entry {
		pos: row.pos,
		published_at: Timestamp::from_second(row.published_at)?,
		payload: SignedEntry::new(
			row.sig.parse()?,
			ScopeEntryBody {
				scope: name.scope().clone(),
				author_identity: row.author_identity,
				payload: YankBody {
					name: name.name().clone(),
					version: Bounded::new(version.clone())?,
					action: row.action,
				},
			},
		),
	}))
}

async fn current_deprecation(
	pool: &MySqlPool,
	package_pos: u64,
	name: &PackageName,
) -> anyhow::Result<Option<Entry<DeprecateScopeEntry>>> {
	let Some(row) = sqlx::query!(
		r#"
		SELECT UNIX_TIMESTAMP(LogEntry.published_at) AS `published_at!`, DeprecateScopeLogEntry.pos, ScopeLogEntry.sig, ScopeLogEntry.author_identity AS `author_identity: IdentityId`, DeprecateScopeLogEntry.reason
		FROM DeprecateScopeLogEntry
		INNER JOIN ScopeLogEntry ON ScopeLogEntry.pos=DeprecateScopeLogEntry.pos
		INNER JOIN LogEntry ON LogEntry.pos=ScopeLogEntry.pos
		WHERE DeprecateScopeLogEntry.package_pos = ?
		ORDER BY DeprecateScopeLogEntry.pos DESC
		LIMIT 1
		"#,
		package_pos,
	)
	.fetch_optional(pool)
	.await?
	else {
		return Ok(None);
	};

	Ok(Some(Entry {
		pos: row.pos,
		published_at: Timestamp::from_second(row.published_at)?,
		payload: SignedEntry::new(
			row.sig.parse()?,
			ScopeEntryBody {
				scope: name.scope().clone(),
				author_identity: row.author_identity,
				payload: DeprecateBody {
					name: name.name().clone(),
					reason: row.reason.parse()?,
				},
			},
		),
	}))
}
