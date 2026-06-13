use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::post;
use actix_web::web;
use pesde::names::PackageName;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::db::ScopeControl;

use crate::AppState;
use crate::api::package::Error;
use crate::shared::auth::WriteGuard;
use crate::shared::db::append_leaf;

#[post("/package/yank")]
pub(super) async fn http_v2(
	_access_guard: WriteGuard,
	app_state: web::Data<AppState>,
	body: web::Json<YankScopeEntry>,
) -> Result<impl Responder, Error> {
	let entry = body.into_inner();

	let package = {
		let body = entry.unsafe_body();
		PackageName::new(body.scope.clone(), body.payload.name.clone())
	};

	handler(app_state.db.as_ref(), entry).await?;

	if let Err(e) = app_state
		.search
		.update(app_state.db.as_ref(), package)
		.await
	{
		tracing::error!("failed to index published package for search: {e:#?}");
	}

	Ok(HttpResponse::Ok().finish())
}

async fn handler(db: &dyn Backend, entry: YankScopeEntry) -> Result<(), Error> {
	let mut store = db.begin_write().await?;
	let author = db
		.author_key(&mut store, &entry.unsafe_body().author_identity)
		.await?
		.ok_or(Error::UnknownIdentity)?;
	let Some((sig, body)) = entry.into_verified_external(&author.key) else {
		return Err(Error::InvalidSignature);
	};

	let Some(access) = db
		.authorize_scope_write(
			&mut store,
			&body.scope,
			&author,
			ScopeControl::Write(&body.payload.name),
		)
		.await?
	else {
		return Err(Error::Unauthorized);
	};

	let (mut store, _) = append_leaf(store, access.pos, &body).await?;
	db.insert_yank(&mut store, access.pos, &sig, &body).await?;
	store.commit().await?;

	Ok(())
}
