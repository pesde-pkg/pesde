use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::names::Name;
use pesde::names::PackageName;
use pesde::names::Scope;
use pesde::source::pesde::registry::*;
use semver::Version;
use sqlx::types::Uuid;

#[get("/v2/package/{scope}/{name}/{version}")]
pub async fn http(
	app_state: web::Data<AppState>,
	path: web::Path<(Scope, Name, Version)>,
) -> ControllerResult {
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
				SELECT LogEntry.seq, ScopeLogEntry.sig, ScopeLogEntry.scope_seq, ScopeLogEntry.author_identity AS `author_identity: Uuid`, PublishScopeLogEntry.archive_hash
				FROM LogEntry
				INNER JOIN ScopeLogEntry ON ScopeLogEntry.seq=LogEntry.seq
				INNER JOIN PublishScopeLogEntry ON PublishScopeLogEntry.seq=ScopeLogEntry.seq
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
				seq: EntrySeq(row.seq),
				payload: EntryPayload::Scope(SignedEntry {
					sig: row.sig.parse()?,
					body: ScopeEntryBody {
						scope: name.scope().clone(),
						scope_seq: ScopeSeq(row.scope_seq),
						author_identity: IdentityId(row.author_identity),
						payload: ScopeEntryPayload::Publish(PublishBody {
							name: name.name().clone(),
							version: version.clone(),
							archive_hash: row.archive_hash.parse()?,
						}),
					},
				}),
			}))
		}
	}
}
