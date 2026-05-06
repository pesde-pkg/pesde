use actix_web::Scope;

pub struct LogController;

impl LogController {
	pub fn v2() -> Scope {
		Scope::new("/log")
			.route("/head", actix_web::web::get().to(Self::head))
			.route("/consistency", actix_web::web::get().to(Self::consistency))
			.route("/inclusion", actix_web::web::get().to(Self::inclusion))
	}

	async fn head() -> actix_web::Result<actix_web::HttpResponse> {
		Ok(actix_web::HttpResponse::Ok().finish())
	}

	async fn consistency() -> actix_web::Result<actix_web::HttpResponse> {
		Ok(actix_web::HttpResponse::Ok().finish())
	}

	async fn inclusion() -> actix_web::Result<actix_web::HttpResponse> {
		Ok(actix_web::HttpResponse::Ok().finish())
	}
}
