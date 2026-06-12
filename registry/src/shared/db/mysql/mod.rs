use std::any::Any;
use std::collections::BTreeMap;

use anyhow::Context as _;
use async_trait::async_trait;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::lock::Mutex;
use futures::stream::BoxStream;
use iter_chunks::IterChunks as _;
use merkleberg::MMRIVER;
use pesde::bounded::BoundedBTreeMap;
use pesde::bounded::BoundedString;
use pesde::bounded::BoundedVec;
use pesde::hash::RawHash;
use pesde::manifest::DependencyType;
use pesde::names::PackageName;
use pesde::names::Scope;
use pesde::signature::KeyKind;
use pesde::signature::PublicKey;
use pesde::source::Realm;
use pesde::source::pesde::registry::*;
use pesde::source::pesde::specifier::RegistryPesdeDependencySpecifier;
use pesde::source::wally::specifier::RegistryWallyDependencySpecifier;
use sqlx::Executor as _;
use sqlx::MySqlConnection;
use sqlx::MySqlExecutor;
use sqlx::MySqlPool;
use sqlx::MySqlTransaction;
use sqlx::QueryBuilder;
use sqlx::mysql::MySqlPoolOptions;
use sqlx::query_builder::Separated;

use crate::shared::db::Backend;
use crate::shared::db::EntryKind;
use crate::shared::db::ReadStore;
use crate::shared::db::ScopeAccess;
use crate::shared::db::ScopeControl;
use crate::shared::db::StoreError;
use crate::shared::db::WriteStore;

mod identity;
mod log;
mod package;
mod scope;

macro_rules! insert_chunked {
	($conn:expr, $table:literal, $columns:expr, $rows:expr, $bind:expr $(,)?) => {
		$crate::shared::db::mysql::insert_chunked_impl(
			$conn,
			const_str::concat!(
				"INSERT INTO ",
				$table,
				" (",
				const_str::join!(&$columns, ", "),
				") "
			),
			const { $columns.len() },
			$rows,
			$bind,
		)
	};
}
pub(super) use insert_chunked;

async fn insert_chunked_impl<T>(
	conn: &mut MySqlConnection,
	prefix: &'static str,
	columns: usize,
	rows: impl IntoIterator<Item = T>,
	mut bind: impl FnMut(Separated<'_, sqlx::MySql, &'static str>, T),
) -> sqlx::Result<()> {
	#[expect(clippy::integer_division)]
	let mut chunks = rows.into_iter().chunks(u16::MAX as usize / columns);
	while let Some(chunk) = chunks.next() {
		let mut query = QueryBuilder::new(prefix);
		query.push_values(chunk, &mut bind);
		query.build().execute(&mut *conn).await?;
	}
	Ok(())
}

pub struct MySqlBackend {
	pool: MySqlPool,
}

impl MySqlBackend {
	pub async fn connect(url: &str) -> Self {
		let pool = MySqlPoolOptions::new()
			.after_connect(|conn, _| {
				Box::pin(async move {
					// a way to ensure that there isn't a race condition in scope write access by avoiding read snapshots in transactions
					// snapshots are not useful for this app anyway; it is append only
					conn.execute("SET SESSION TRANSACTION ISOLATION LEVEL READ COMMITTED")
						.await?;
					Ok(())
				})
			})
			.connect(url)
			.await
			.expect("failed to connect to mysql database");

		sqlx::migrate!()
			.run(&pool)
			.await
			.expect("failed to migrate mysql database");

		Self { pool }
	}
}

#[async_trait]
impl Backend for MySqlBackend {
	async fn current_size(&self) -> anyhow::Result<u64> {
		Ok(sqlx::query!("SELECT size FROM Tree")
			.fetch_one(&self.pool)
			.await?
			.size)
	}

	async fn all_packages_for_index(&self) -> BoxStream<'_, anyhow::Result<(PackageName, String)>> {
		sqlx::query!(
			r#"
			SELECT Scope.scope, Package.name, latest.description
			FROM Package
			INNER JOIN Scope ON Scope.id=Package.scope_id
			INNER JOIN PublishScopeLogEntry latest ON latest.pos = (
				SELECT pos FROM PublishScopeLogEntry WHERE package_pos = Package.genesis_pos ORDER BY pos DESC LIMIT 1
			)
			"#,
		)
		.fetch(&self.pool)
		.map(|row| {
			let row = row?;
			Ok((
				PackageName::new(row.scope.parse()?, row.name.parse()?),
				row.description
			))
		})
		.boxed()
	}

	async fn read_mmr_at(
		&self,
		size: u64,
	) -> anyhow::Result<MMRIVER<CurrentMmrMerge, Box<dyn ReadStore>>> {
		Ok(MMRIVER::new(
			size,
			Box::new(MySqlReadStore {
				pool: self.pool.clone(),
			}),
		))
	}

	async fn begin_write(&self) -> anyhow::Result<Box<dyn WriteStore>> {
		let tx = self.pool.begin().await?;
		Ok(Box::new(MySqlWriteStore { tx: Mutex::new(tx) }))
	}

	async fn current_identity_key(
		&self,
		store: &mut Box<dyn WriteStore>,
		id: &IdentityId,
	) -> anyhow::Result<Option<PublicKey>> {
		let Some(row) = sqlx::query!(
			r#"
			SELECT algorithm AS `algorithm: KeyKind`, public_key
			FROM IdentityKeyEntry
			WHERE identity_id = ?
			ORDER BY pos DESC
			LIMIT 1
			"#,
			id,
		)
		.fetch_optional(&mut **as_tx(store))
		.await?
		else {
			return Ok(None);
		};

		build_public_key(row.algorithm, row.public_key).map(Some)
	}

	async fn lock_tree(&self, store: &mut Box<dyn WriteStore>) -> anyhow::Result<u64> {
		Ok(sqlx::query!("SELECT size FROM Tree FOR UPDATE")
			.fetch_one(&mut **as_tx(store))
			.await?
			.size)
	}

	async fn scope_write_access(
		&self,
		store: &mut Box<dyn WriteStore>,
		scope: &Scope,
		identity: &IdentityId,
		control: ScopeControl<'_>,
	) -> anyhow::Result<Option<ScopeAccess>> {
		// we need to be careful to avoid locking the table until we're almost certain
		// otherwise, bad actors could lock the app by simply spamming the API with invalid data

		let (package, owner_only, allow_absent) = match control {
			ScopeControl::Write(package) => (package.as_str(), false, false),
			ScopeControl::PublishOrCreate(package) => (package.as_str(), false, true),
			ScopeControl::Owner => ("", true, false),
		};

		let check = async |store: &mut Box<dyn WriteStore>| {
			let row = sqlx::query!(
				r#"
				SELECT
				(
					EXISTS (
						SELECT 1 FROM Scope
						INNER JOIN ScopeManifest ON ScopeManifest.scope_id=Scope.id
						LEFT JOIN ScopeManifestMember ON ScopeManifestMember.pos=ScopeManifest.pos
						WHERE ScopeManifest.pos = (SELECT pos FROM ScopeManifest sm WHERE sm.scope_id = Scope.id ORDER BY pos DESC LIMIT 1)
						  AND Scope.scope = ? 
						  AND (ScopeManifest.owner = ?
						   OR (NOT ? AND ScopeManifestMember.identity_id = ? AND ScopeManifestMember.package IN ('', ?)))
					)
					OR
					(? AND NOT EXISTS (SELECT 1 FROM Scope WHERE Scope.scope = ?))
				) AS `authorized: bool`,
				EXISTS (
					SELECT 1 FROM Scope
					WHERE Scope.scope = ?
				) AS `scope_exists: bool`
				"#,
				scope.as_str(),
				identity,
				owner_only,
				identity,
				package,
				allow_absent,
				scope.as_str(),
				scope.as_str(),
			)
			.fetch_one(&mut **as_tx(store))
			.await?;

			Ok::<_, anyhow::Error>(
				row.authorized
					.unwrap_or_default()
					.then_some(row.scope_exists),
			)
		};

		if check(store).await?.is_none() {
			return Ok(None);
		}

		let pos = self.lock_tree(store).await?;

		Ok(check(store)
			.await?
			.map(|scope_exists| ScopeAccess { pos, scope_exists }))
	}
}

struct MySqlReadStore {
	pool: MySqlPool,
}

#[async_trait]
impl ReadStore for MySqlReadStore {
	async fn get_node(&self, pos: u64) -> Result<Option<RawHash>, StoreError> {
		get_hash(&self.pool, pos)
			.await
			.map_err(|e| StoreError(e.into()))
	}
}

struct MySqlWriteStore {
	tx: Mutex<MySqlTransaction<'static>>,
}

#[async_trait]
impl WriteStore for MySqlWriteStore {
	async fn get_node(&self, pos: u64) -> Result<Option<RawHash>, StoreError> {
		get_hash(&mut **self.tx.lock().await, pos)
			.await
			.map_err(|e| StoreError(e.into()))
	}

	async fn append_nodes(&mut self, pos: u64, elems: Vec<RawHash>) -> Result<(), StoreError> {
		insert_chunked!(
			self.tx.get_mut(),
			"TreeNode",
			["pos", "sha256"],
			(pos..).zip(elems),
			|mut b, (i, elem)| {
				b.push_bind(i).push_bind(elem);
			},
		)
		.await
		.map_err(|e| StoreError(e.into()))
	}

	async fn set_size(&mut self, size: u64) -> anyhow::Result<()> {
		sqlx::query!("UPDATE Tree SET size = ?", size)
			.execute(&mut **self.tx.get_mut())
			.await?;
		Ok(())
	}

	async fn commit(self: Box<Self>) -> anyhow::Result<()> {
		self.tx.into_inner().commit().await?;
		Ok(())
	}
}

fn as_tx(store: &mut Box<dyn WriteStore>) -> &mut MySqlTransaction<'static> {
	(&mut **store as &mut dyn Any)
		.downcast_mut::<MySqlWriteStore>()
		.expect("write store does not belong to the mysql backend")
		.tx
		.get_mut()
}

async fn get_hash(executor: impl MySqlExecutor<'_>, pos: u64) -> sqlx::Result<Option<RawHash>> {
	Ok(sqlx::query!(
		"SELECT sha256 AS `sha256: RawHash` FROM TreeNode WHERE pos = ?",
		pos
	)
	.fetch_optional(executor)
	.await?
	.map(|record| record.sha256))
}

#[derive(sqlx::Type, Debug)]
#[sqlx(rename_all = "snake_case")]
pub(super) enum ScopeEntryKind {
	Publish,
	Yank,
	Deprecate,
	ManifestUpdate,
}

#[derive(sqlx::Type, Debug, Clone, Copy)]
#[sqlx(rename_all = "snake_case")]
pub(super) enum DependencyKind {
	Pesde,
	Wally,
}

fn build_public_key(kind: KeyKind, public_key: Vec<u8>) -> anyhow::Result<PublicKey> {
	PublicKey::new(kind, public_key)
		.ok_or_else(|| anyhow::anyhow!("stored public key has an invalid length"))
}

#[allow(clippy::too_many_arguments)]
async fn build_publish_body(
	pool: &MySqlPool,
	pos: u64,
	name: &str,
	version: &str,
	archive_hash: &str,
	description: String,
	license: String,
	repository: &str,
) -> anyhow::Result<PublishBody> {
	let mut authors_stream = sqlx::query!(
		r#"
		SELECT author
		FROM PublishAuthor
		WHERE pos = ?
		ORDER BY seq
		"#,
		pos
	)
	.fetch(pool);

	let mut authors = vec![];
	while let Some(row) = authors_stream.try_next().await? {
		authors.push(BoundedString::new(row.author)?);
	}

	let mut dependencies_stream = sqlx::query!(
		r#"
		SELECT alias, dependency_type AS `dependency_type: DependencyType`, kind AS `kind: DependencyKind`, name, version_req, registry, realm AS `realm: Realm`
		FROM PublishDependency
		WHERE pos = ?
		"#,
		pos,
	)
	.fetch(pool);

	let mut dependencies = BTreeMap::new();
	while let Some(row) = dependencies_stream.try_next().await? {
		let specifier = match row.kind {
			DependencyKind::Pesde => {
				RegistryDependencySpecifier::Pesde(RegistryPesdeDependencySpecifier {
					name: row.name.parse()?,
					version: row.version_req.parse()?,
					registry: row.registry.as_deref().map(str::parse).transpose()?,
					realm: row.realm,
				})
			}
			DependencyKind::Wally => {
				RegistryDependencySpecifier::Wally(RegistryWallyDependencySpecifier {
					name: row.name.parse()?,
					version: row.version_req.parse()?,
					index: row
						.registry
						.context("wally dependency missing index url")?
						.parse()?,
					realm: row.realm.context("wally dependency missing realm")?,
				})
			}
		};
		dependencies.insert(row.alias.parse()?, (specifier, row.dependency_type));
	}

	Ok(PublishBody {
		name: name.parse()?,
		version: version.parse()?,
		archive_hash: archive_hash.parse()?,
		description: BoundedString::new(description)?,
		license: BoundedString::new(license)?,
		authors: BoundedVec::new(authors)?,
		repository: Some(repository)
			.filter(|r| !r.is_empty())
			.map(str::parse)
			.transpose()?,
		dependencies: BoundedBTreeMap::new(dependencies)?,
	})
}

pub(super) async fn insert_log_entry(
	tx: &mut MySqlTransaction<'_>,
	pos: u64,
	kind: EntryKind,
) -> sqlx::Result<()> {
	sqlx::query!("INSERT INTO LogEntry (pos, kind) VALUES (?, ?)", pos, kind)
		.execute(&mut **tx)
		.await?;
	Ok(())
}

async fn insert_scope_envelope<P>(
	tx: &mut MySqlTransaction<'_>,
	pos: u64,
	sig: &pesde::signature::Signature,
	body: &ScopeEntryBody<P>,
	kind: ScopeEntryKind,
) -> anyhow::Result<u64> {
	let scope = body.scope.as_str();
	let scope_id = if let Some(row) = sqlx::query!("SELECT id FROM Scope WHERE scope = ?", scope)
		.fetch_optional(&mut **tx)
		.await?
	{
		row.id
	} else {
		sqlx::query!("INSERT INTO Scope (scope) VALUES (?)", scope)
			.execute(&mut **tx)
			.await?
			.last_insert_id()
	};
	insert_log_entry(tx, pos, EntryKind::Scope).await?;

	sqlx::query!(
		"INSERT INTO ScopeLogEntry (pos, sig, scope_id, author_identity, kind) VALUES (?, ?, ?, ?, ?)",
		pos,
		sig.to_string(),
		scope_id,
		&body.author_identity,
		kind,
	)
	.execute(&mut **tx)
	.await?;

	Ok(scope_id)
}
