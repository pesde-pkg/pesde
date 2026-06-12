mod deprecate;
mod error;
mod get_archive;
mod get_package;
mod get_readme;
mod get_version;
mod get_versions;
mod publish;
mod yank;

use async_trait::async_trait;
use pesde::names::PackageName;
use pesde::signature::Signature;
use pesde::source::pesde::registry::*;
use semver::Version;

use crate::shared::db::PackageWriteError;
use crate::shared::db::WriteStore;

pub use error::Error;

pub fn http_v2(cfg: &mut actix_web::web::ServiceConfig) {
	cfg.service(deprecate::http_v2)
		.service(get_archive::http_v2)
		.service(get_readme::http_v2)
		.service(get_versions::http_v2)
		.service(get_package::http_v2)
		.service(get_version::http_v2)
		.service(publish::http_v2)
		.service(yank::http_v2);
}

#[async_trait]
pub trait Repository {
	async fn package_version(
		&self,
		name: &PackageName,
		version: &Version,
	) -> anyhow::Result<Option<PackageVersionResponse>>;

	async fn package_info(&self, name: &PackageName)
	-> anyhow::Result<Option<PackageInfoResponse>>;

	async fn package_versions(
		&self,
		name: &PackageName,
		after: u64,
		limit: u8,
	) -> anyhow::Result<PackageVersionsResponse>;

	async fn insert_publish(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &ScopeEntryBody<PublishBody>,
	) -> Result<(), PackageWriteError>;

	async fn insert_yank(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &ScopeEntryBody<YankBody>,
	) -> Result<(), PackageWriteError>;

	async fn insert_deprecate(
		&self,
		tx: &mut Box<dyn WriteStore>,
		pos: u64,
		sig: &Signature,
		body: &ScopeEntryBody<DeprecateBody>,
	) -> Result<(), PackageWriteError>;
}
