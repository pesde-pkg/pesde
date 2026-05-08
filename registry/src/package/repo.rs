use std::path::PathBuf;

use async_trait::async_trait;
use pesde::names::PackageName;
use semver::Version;
use sqlx::SqlitePool;

#[async_trait]
pub trait PackageRepo: Send + Sync + 'static {
	/// Gets the versions of a package
	async fn versions(&self, name: &PackageName) -> anyhow::Result<()>;

	/// Gets the version of a package
	async fn version(&self, name: &PackageName, version: &Version) -> anyhow::Result<()>;
}

#[async_trait]
pub trait PackageArchiveRepo: Send + Sync + 'static {
	/// Gets the archive for a package version
	async fn archive(&self, name: &PackageName, version: &Version) -> anyhow::Result<()>;
}

pub struct SqlitePackageRepo {
	pool: SqlitePool,
}

impl SqlitePackageRepo {
	pub fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}
}

#[async_trait]
impl PackageRepo for SqlitePackageRepo {
	async fn versions(&self, name: &PackageName) -> anyhow::Result<()> {
		todo!()
	}

	async fn version(&self, name: &PackageName, version: &Version) -> anyhow::Result<()> {
		todo!()
	}
}

pub struct FsPackageArchiveRepo {
	path: PathBuf,
}

impl FsPackageArchiveRepo {
	pub fn new(path: PathBuf) -> Self {
		Self { path }
	}
}

#[async_trait]
impl PackageArchiveRepo for FsPackageArchiveRepo {
	async fn archive(&self, name: &PackageName, version: &Version) -> anyhow::Result<()> {
		todo!()
	}
}
