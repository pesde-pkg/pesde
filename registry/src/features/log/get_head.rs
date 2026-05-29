use std::num::NonZeroU64;

use crate::AppState;
use crate::shared::db::Database;
use crate::util::AppResult;
use crate::util::ControllerResult;
use actix_web::HttpResponse;
use actix_web::get;
use actix_web::web;
use pesde::source::pesde::registry::*;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ConsistencyQuery {
	size_from: Option<NonZeroU64>,
}

#[get("/v2/log/head")]
pub async fn http(
	app_state: web::Data<AppState>,
	query: web::Query<ConsistencyQuery>,
) -> ControllerResult {
	let Some(head) = handler(&app_state.database, query.size_from).await? else {
		return Ok(HttpResponse::NotFound().finish());
	};

	Ok(HttpResponse::Ok().json(head))
}

async fn handler(
	db: &Database,
	size_from: Option<NonZeroU64>,
) -> AppResult<Option<LogHeadResponse>> {
	let Some((seq, mmr_size)) = query(db).await? else {
		return Ok(None);
	};

	let mmr = db.read_mmr_sized(mmr_size);

	Ok(Some(LogHeadResponse {
		seq,
		accumulator: mmr.get_accumulator().await?,
		state: match size_from {
			Some(size_from) => LogHeadResponseState::WithPreviousState {
				proof: mmr.gen_consistency_proof(size_from.get()).await?,
			},
			None => LogHeadResponseState::OnlyNewState {
				mmr_size_to: mmr_size,
			},
		},
	}))
}

async fn query(db: &Database) -> anyhow::Result<Option<(EntrySeq, u64)>> {
	match db {
		Database::MySql(pool) => {
			let Some(record) = sqlx::query!(
				r#"
				SELECT seq, COUNT(TreeNode.pos) AS `mmr_size: u64`
				FROM TreeNode
				INNER JOIN LogEntry ON LogEntry.pos=TreeNode.pos
				ORDER BY seq DESC
				LIMIT 1
				"#
			)
			.fetch_optional(pool)
			.await?
			else {
				return Ok(None);
			};

			Ok(record.seq.map(|seq| (EntrySeq(seq), record.mmr_size)))
		}
	}
}
