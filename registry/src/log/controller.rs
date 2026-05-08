use actix_web::Scope;
use actix_web::web;
use pesde::source::pesde::backend::EntrySeq;
use thiserror::Error;

use crate::AppState;
use crate::log::service::LogService;
use crate::util::ControllerResult;

pub struct LogController;

impl LogController {
	pub fn v2() -> Scope {
		Scope::new("/log")
			.route("/head", actix_web::web::get().to(Self::head))
			.route("/consistency", actix_web::web::get().to(Self::consistency))
			.route("/inclusion", actix_web::web::get().to(Self::inclusion))
			.route("/entry/{seq}", actix_web::web::get().to(Self::entry))
	}

	async fn head() -> ControllerResult {
		Ok(actix_web::HttpResponse::Ok().finish())
	}

	async fn consistency() -> ControllerResult {
		Ok(actix_web::HttpResponse::Ok().finish())
	}

	async fn inclusion() -> ControllerResult {
		Ok(actix_web::HttpResponse::Ok().finish())
	}

	async fn entry(app_state: web::Data<AppState>, seq: web::Path<EntrySeq>) -> ControllerResult {
		let Some(entry) = LogService::entry(&app_state.repos, seq.into_inner()).await? else {
			return Ok(actix_web::HttpResponse::NotFound().finish());
		};

		Ok(actix_web::HttpResponse::Ok().json(entry))
	}
}

#[derive(Debug, Error)]
pub enum LogControllerError {}
