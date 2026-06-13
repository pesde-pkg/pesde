use async_trait::async_trait;
use futures::StreamExt as _;
use futures::stream::BoxStream;
use jiff::Timestamp;
use pesde::names::PackageName;
use pesde::source::pesde::registry::SearchResultItem;
use pesde_registry_core::features::search::Repository;
use pesde_registry_core::features::search::SearchPackage;

use crate::MySqlBackend;

#[async_trait]
impl Repository for MySqlBackend {
	async fn all_packages_for_index(&self) -> BoxStream<'_, anyhow::Result<SearchPackage>> {
		sqlx::query!(
			r#"
			SELECT Package.genesis_pos, PublishScopeLogEntry.pos, Scope.scope, Package.name, PublishScopeLogEntry.version, PublishScopeLogEntry.description, UNIX_TIMESTAMP(LogEntry.published_at) AS `published_at!`
			FROM PublishScopeLogEntry
			INNER JOIN Package ON Package.genesis_pos=PublishScopeLogEntry.package_pos
			INNER JOIN Scope ON Scope.id=Package.scope_id
            INNER JOIN LogEntry ON LogEntry.pos=PublishScopeLogEntry.pos
            WHERE PublishScopeLogEntry.pos = (
                SELECT PublishScopeLogEntry.pos
                FROM PublishScopeLogEntry
                LEFT JOIN YankScopeLogEntry ON YankScopeLogEntry.publish_pos=PublishScopeLogEntry.pos AND YankScopeLogEntry.pos=(SELECT pos FROM YankScopeLogEntry WHERE publish_pos=PublishScopeLogEntry.pos ORDER BY pos DESC LIMIT 1)
                WHERE PublishScopeLogEntry.package_pos=Package.genesis_pos
    			ORDER BY (YankScopeLogEntry.action IS NULL OR YankScopeLogEntry.action = 'revoke') DESC, PublishScopeLogEntry.version_ord DESC
                LIMIT 1
            )
			"#,
		)
		.fetch(&self.pool)
		.map(|row| {
			let row = row?;
			Ok(SearchPackage {
                id: row.genesis_pos,
                pos: row.pos,
                item: SearchResultItem {
                    name: PackageName::new(row.scope.parse()?, row.name.parse()?),
                    version: row.version.parse()?,
                    published_at: Timestamp::from_second(row.published_at)?,
                    description: row.description
                },
            })
		})
		.boxed()
	}

	async fn searchable_version(&self, name: &PackageName) -> anyhow::Result<SearchPackage> {
		let row = sqlx::query!(
			r#"
            SELECT Package.genesis_pos, PublishScopeLogEntry.pos, PublishScopeLogEntry.version, PublishScopeLogEntry.description, UNIX_TIMESTAMP(LogEntry.published_at) AS `published_at!`
            FROM PublishScopeLogEntry
            INNER JOIN Package ON Package.genesis_pos=PublishScopeLogEntry.package_pos
			INNER JOIN Scope ON Scope.id=Package.scope_id
            INNER JOIN LogEntry ON LogEntry.pos=PublishScopeLogEntry.pos
            LEFT JOIN YankScopeLogEntry ON YankScopeLogEntry.publish_pos=PublishScopeLogEntry.pos AND YankScopeLogEntry.pos=(SELECT pos FROM YankScopeLogEntry WHERE publish_pos=PublishScopeLogEntry.pos ORDER BY pos DESC LIMIT 1)
            WHERE Scope.scope = ? AND Package.name = ?
            ORDER BY (YankScopeLogEntry.action IS NULL OR YankScopeLogEntry.action = 'revoke') DESC, PublishScopeLogEntry.version_ord DESC
            LIMIT 1
            "#,
            name.scope().as_str(),
            name.name().as_str(),
		)
        .fetch_one(&self.pool)
        .await?;

		Ok(SearchPackage {
			id: row.genesis_pos,
			pos: row.pos,
			item: SearchResultItem {
				name: name.clone(),
				version: row.version.parse()?,
				published_at: Timestamp::from_second(row.published_at)?,
				description: row.description,
			},
		})
	}

	async fn search_result_by_pos(&self, pos: u64) -> anyhow::Result<SearchResultItem> {
		let row = sqlx::query!(
			r#"
            SELECT Scope.scope, Package.name, PublishScopeLogEntry.version, PublishScopeLogEntry.description, UNIX_TIMESTAMP(LogEntry.published_at) AS `published_at!`
            FROM PublishScopeLogEntry
            INNER JOIN Package ON Package.genesis_pos=PublishScopeLogEntry.package_pos
			INNER JOIN Scope ON Scope.id=Package.scope_id
            INNER JOIN LogEntry ON LogEntry.pos=PublishScopeLogEntry.pos
            LEFT JOIN YankScopeLogEntry ON YankScopeLogEntry.publish_pos=PublishScopeLogEntry.pos AND YankScopeLogEntry.pos=(SELECT pos FROM YankScopeLogEntry WHERE publish_pos=PublishScopeLogEntry.pos ORDER BY pos DESC LIMIT 1)
            WHERE PublishScopeLogEntry.pos = ?
            ORDER BY (YankScopeLogEntry.action IS NULL OR YankScopeLogEntry.action = 'revoke') DESC, PublishScopeLogEntry.version_ord DESC
            LIMIT 1
            "#,
            pos
		)
        .fetch_one(&self.pool)
        .await?;

		Ok(SearchResultItem {
			name: PackageName::new(row.scope.parse()?, row.name.parse()?),
			version: row.version.parse()?,
			published_at: Timestamp::from_second(row.published_at)?,
			description: row.description,
		})
	}
}
