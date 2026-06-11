use actix_web::Responder;
use actix_web::get;
use actix_web::web;
use actix_web_lab::respond::NdJson;
use futures::StreamExt as _;
use futures::stream::BoxStream;
use pesde::names::Name;
use pesde::names::PackageName;
use pesde::names::Scope;
use pesde::source::pesde::registry::Entry;
use pesde::source::pesde::registry::PublishScopeEntry;

use crate::AppState;
use crate::features::package::Error;
use crate::shared::auth::ReadGuard;
use crate::shared::db::Backend;

#[get("/package/{scope}/{name}")]
pub(super) async fn http_v2(
	_access_guard: ReadGuard,
	app_state: web::Data<AppState>,
	path: web::Path<(Scope, Name)>,
) -> Result<impl Responder, Error> {
	let (scope, name) = path.into_inner();
	let package_name = PackageName::new(scope, name);

	let entries = handler(app_state.db.as_ref(), &package_name)
		.await
		.map(|entry| entry.map_err(std::io::Error::other));

	Ok(NdJson::new(entries).into_responder())
}

async fn handler(
	db: &dyn Backend,
	name: &PackageName,
) -> BoxStream<'static, anyhow::Result<Entry<PublishScopeEntry>>> {
	db.package_versions(name).await
}
