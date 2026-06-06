use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::HttpResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::names::Name;
use pesde::names::PackageName;
use pesde::names::Scope;
use pesde::source::pesde::registry::*;
use semver::Version;
use sqlx::types::Uuid;

#[get("/package/{scope}/{name}/{version}")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	path: web::Path<(Scope, Name, Version)>,
) -> HttpResult {
	let (scope, name, version) = path.into_inner();
	let package_name = PackageName::new(scope, name);

	let Some(entry) = handler(&app_state.database, package_name, version).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(db: &Database, name: PackageName, version: Version) -> AppResult<Option<Entry>> {
	query(db, name, version).await.map_err(Into::into)
}

async fn query(
	db: &Database,
	name: PackageName,
	version: Version,
) -> anyhow::Result<Option<Entry>> {
	match db {
		Database::MySql(pool) => {
			let Some(row) = sqlx::query!(
				r#"
				SELECT LogEntry.pos, ScopeLogEntry.sig, ScopeLogEntry.author_identity AS `author_identity: Uuid`, PublishScopeLogEntry.archive_hash
				FROM LogEntry
				INNER JOIN ScopeLogEntry ON ScopeLogEntry.pos=LogEntry.pos
				INNER JOIN PublishScopeLogEntry ON PublishScopeLogEntry.pos=ScopeLogEntry.pos
				WHERE ScopeLogEntry.scope = ? AND PublishScopeLogEntry.name = ? AND PublishScopeLogEntry.version = ?
				"#,
				name.scope().as_str(),
				name.name().as_str(),
				version.to_string(),
			)
			.fetch_optional(pool).await? else {
				return Ok(None);
			};

			Ok(Some(Entry {
				pos: row.pos,
				payload: EntryPayload::Scope(SignedEntry::new(
					row.sig.parse()?,
					ScopeEntryBody {
						scope: name.scope().clone(),
						author_identity: IdentityId(row.author_identity),
						payload: ScopeEntryPayload::Publish(PublishBody {
							name: name.name().clone(),
							version: version.clone(),
							archive_hash: row.archive_hash.parse()?,
						}),
					},
				)),
			}))
		}
	}
}
