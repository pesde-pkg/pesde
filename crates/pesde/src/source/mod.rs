#![expect(deprecated)]
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::Url;
use crate::registry::manifest::Alias;
use crate::registry::manifest::DependencyType;
use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::link::PackageStructure;
use crate::reporters::DownloadProgressReporter;
use crate::vfs::PackageStorer;

mod id;
pub use id::*;
mod specifier;
pub use specifier::*;
mod workspace;
pub use workspace::*;

pub type SourceError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[async_trait]
pub trait PackageSource: Send + Sync {
	async fn resolve(&self, specifier: &DependencySpecifier) -> Result<ResolveResult, SourceError>;

	async fn download(
		&self,
		package_id: &PackageId,
		reporter: Arc<dyn DownloadProgressReporter + 'static>,
		to: &mut dyn PackageStorer,
	) -> Result<(), SourceError>;

	async fn inspect(&self, package_id: &PackageId) -> Result<InspectResult, SourceError>;
}

/// A provider of [PackageSource]s. This is used to provide the source implementations.
#[async_trait]
pub trait SourceProvider: Send + Sync {
	async fn get(&self, source_id: &SourceId) -> Result<Arc<dyn PackageSource>, SourceError>;
}

pub enum ResolveResult {
	/// This source was able to provide candidates for the given specifier.
	Found(BoxStream<'static, Result<(PackageId, Option<InspectResult>), SourceError>>),
	/// This source doesn't contain candidates, but it knows other sources that might.
	Elsewhere(Box<[SourceId]>),
}

pub struct InspectResult {
	pub structure: PackageStructure,
	pub dependencies: BTreeMap<Alias, (DependencySpecifier, DependencyType)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceId {
	Pesde(Url),
	Wally(Url),
	LegacyPesde(Url),
	Git(Url),
	Workspace,
}
