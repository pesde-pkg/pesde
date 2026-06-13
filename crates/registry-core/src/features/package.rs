use std::num::NonZero;

use async_trait::async_trait;
use pesde::names::PackageName;
use pesde::signature::Signature;
use pesde::source::pesde::registry::DeprecateBody;
use pesde::source::pesde::registry::PackageInfoResponse;
use pesde::source::pesde::registry::PackageVersionResponse;
use pesde::source::pesde::registry::PackageVersionsResponse;
use pesde::source::pesde::registry::PublishBody;
use pesde::source::pesde::registry::ScopeEntryBody;
use pesde::source::pesde::registry::YankBody;
use semver::Version;

use crate::db::WriteStore;

#[derive(Debug, thiserror::Error)]
pub enum PackageWriteError {
	#[error("the package version has already been published")]
	VersionAlreadyExists,

	#[error("the package version does not exist")]
	UnknownPackageVersion,

	#[error("the package version is already yanked")]
	AlreadyYanked,

	#[error("the package version is not yanked")]
	NotYanked,

	#[error("the package is already deprecated")]
	AlreadyDeprecated,

	#[error("the package is not deprecated")]
	NotDeprecated,

	#[error(transparent)]
	Internal(#[from] anyhow::Error),
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
		limit: NonZero<u8>,
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
