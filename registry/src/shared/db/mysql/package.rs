use async_trait::async_trait;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::stream::BoxStream;
use pesde::names::PackageName;
use pesde::signature::Signature;
use pesde::source::pesde::registry::*;
use semver::Version;
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

#[async_trait]
impl Repository for MySqlBackend {
	async fn package_version(
		&self,
		name: &PackageName,
		version: &Version,
	) -> anyhow::Result<Option<Entry<PublishScopeEntry>>> {
		let Some(row) = sqlx::query!(
			r#"
            SELECT ScopeLogEntry.pos, ScopeLogEntry.sig, ScopeLogEntry.author_identity AS `author_identity: IdentityId`,
                   Package.name,
				   PublishScopeLogEntry.version, PublishScopeLogEntry.archive_hash, PublishScopeLogEntry.description, PublishScopeLogEntry.license, PublishScopeLogEntry.repository
            FROM ScopeLogEntry
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
			row.repository.as_deref(),
		)
		.await?;

		Ok(Some(Entry {
			pos: row.pos,
			payload: SignedEntry::new(
				row.sig.parse()?,
				ScopeEntryBody {
					scope: name.scope().clone(),
					author_identity: row.author_identity,
					payload: publish,
				},
			),
		}))
	}

	async fn package_versions(
		&self,
		name: &PackageName,
	) -> BoxStream<'static, anyhow::Result<Entry<PublishScopeEntry>>> {
		let pool = self.pool.clone();
		let scope = name.scope().clone();
		let name = name.name().clone();

		async_stream::try_stream! {
			let mut rows = sqlx::query!(
				r#"
				SELECT ScopeLogEntry.pos, ScopeLogEntry.sig, ScopeLogEntry.author_identity AS `author_identity: IdentityId`,
					Package.name,
					PublishScopeLogEntry.version, PublishScopeLogEntry.archive_hash, PublishScopeLogEntry.description, PublishScopeLogEntry.license, PublishScopeLogEntry.repository
				FROM ScopeLogEntry
				INNER JOIN PublishScopeLogEntry ON PublishScopeLogEntry.pos=ScopeLogEntry.pos
				INNER JOIN Package ON Package.genesis_pos=PublishScopeLogEntry.package_pos
				INNER JOIN Scope ON Scope.id=Package.scope_id
				WHERE Scope.scope = ? AND Package.name = ?
				ORDER BY ScopeLogEntry.pos
                "#,
				scope.as_str(),
				name.as_str(),
			)
			.fetch(&pool);

			while let Some(row) = rows.try_next().await? {
				let publish = build_publish_body(
					&pool,
					row.pos,
					&row.name,
					&row.version,
					&row.archive_hash,
					row.description,
					row.license,
					row.repository.as_deref(),
				)
				.await?;

				yield Entry {
					pos: row.pos,
					payload: SignedEntry::new(
						row.sig.parse()?,
						ScopeEntryBody {
							scope: scope.clone(),
							author_identity: row.author_identity,
							payload: publish,
						},
					),
				};
			}
		}
		.boxed()
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
			"INSERT INTO PublishScopeLogEntry (pos, package_pos, version, archive_hash, description, license, repository) VALUES (?, ?, ?, ?, ?, ?, ?)",
			pos,
			package_pos,
			publish.version.to_string(),
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
		let yank = &body.payload;
		let conn = as_tx(tx);

		let Some(publish_pos) = sqlx::query!(
			r#"
            SELECT PublishScopeLogEntry.pos
			FROM PublishScopeLogEntry
            INNER JOIN Package ON Package.genesis_pos=PublishScopeLogEntry.package_pos
            INNER JOIN Scope ON Scope.id=Package.scope_id
            WHERE Scope.scope = ? AND Package.name = ? AND PublishScopeLogEntry.version = ?
            "#,
			body.scope.as_str(),
			yank.name.as_str(),
			yank.version.to_string(),
		)
		.fetch_optional(&mut **conn)
		.await
		.map_err(anyhow::Error::from)?
		.map(|row| row.pos) else {
			return Err(PackageWriteError::UnknownPackageVersion);
		};

		insert_scope_envelope(conn, pos, sig, body, ScopeEntryKind::Yank).await?;

		match sqlx::query!(
			"INSERT INTO YankScopeLogEntry (pos, publish_pos) VALUES (?, ?)",
			pos,
			publish_pos,
		)
		.execute(&mut **conn)
		.await
		{
			Ok(_) => {}
			Err(e)
				if e.as_database_error()
					.is_some_and(DatabaseError::is_unique_violation) =>
			{
				return Err(PackageWriteError::AlreadyYanked);
			}
			Err(e) => return Err(anyhow::Error::from(e).into()),
		}

		Ok(())
	}

	async fn insert_deprecate(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &ScopeEntryBody<DeprecateBody>,
	) -> Result<(), PackageWriteError> {
		let deprecate = &body.payload;
		let conn = as_tx(tx);

		let Some(package_pos) = sqlx::query!(
			r#"
            SELECT Package.genesis_pos
			FROM Package
            INNER JOIN Scope ON Scope.id=Package.scope_id
            WHERE Scope.scope = ? AND Package.name = ?
            "#,
			body.scope.as_str(),
			deprecate.name.as_str(),
		)
		.fetch_optional(&mut **conn)
		.await
		.map_err(anyhow::Error::from)?
		.map(|row| row.genesis_pos) else {
			return Err(PackageWriteError::UnknownPackageVersion);
		};

		insert_scope_envelope(conn, pos, sig, body, ScopeEntryKind::Deprecate).await?;

		sqlx::query!(
			"INSERT INTO DeprecateScopeLogEntry (pos, package_pos, reason) VALUES (?, ?, ?)",
			pos,
			package_pos,
			deprecate.reason.as_str(),
		)
		.execute(&mut **conn)
		.await
		.map_err(anyhow::Error::from)?;

		Ok(())
	}
}
