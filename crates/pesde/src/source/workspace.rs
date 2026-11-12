use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::stream;

use crate::Manifest;
use crate::Workspace;
use crate::link::PackageStructure;
use crate::reporters::DownloadProgressReporter;
use crate::source::DependencySpecifier;
use crate::source::InspectResult;
use crate::source::PackageId;
use crate::source::PackageSource;
use crate::source::ResolveResult;
use crate::source::SourceError;
use crate::vfs::PackageStorer;

pub struct WorkspacePackageSource {
	workspace: Workspace,
}

impl WorkspacePackageSource {
	#[must_use]
	pub fn new(workspace: Workspace) -> Self {
		Self { workspace }
	}
}

#[async_trait]
impl PackageSource for WorkspacePackageSource {
	async fn resolve(&self, specifier: &DependencySpecifier) -> Result<ResolveResult, SourceError> {
		let DependencySpecifier::Workspace(specifier) = specifier else {
			return Err(
				format!("invalid dependency specifier for workspace source: {specifier}").into(),
			);
		};

		let mut members = self.workspace.members_with_manifests().await;
		let mut member = None;
		while let Some(result) = members.join_next().await {
			let result = result??;
			if result.1.package.name == specifier.name {
				member = Some(result);
				// don't break here, the other members will also check for duplicates
			}
		}

		let (member, manifest) =
			member.ok_or_else(|| format!("workspace member `{}` not found", specifier.name))?;

		Ok(ResolveResult::Found(
			stream::once(std::future::ready(Ok((
				PackageId::Workspace(member.into_parts().1),
				Some(Self::inspect(&manifest)?),
			))))
			.boxed(),
		))
	}

	async fn download(
		&self,
		_package_id: &PackageId,
		_reporter: Arc<dyn DownloadProgressReporter + 'static>,
		_to: &mut dyn PackageStorer,
	) -> Result<(), SourceError> {
		Err("download not supported for workspace source, the Vfs should link instead".into())
	}

	async fn inspect(&self, package_id: &PackageId) -> Result<InspectResult, SourceError> {
		let PackageId::Workspace(importer) = package_id else {
			return Err(format!("invalid package id for workspace source: {package_id}").into());
		};

		let member = self.workspace.clone().member(importer.clone());
		let manifest = member.deser_manifest().await?;

		Ok(Self::inspect(&manifest)
			.map_err(|e| format!("failed to inspect workspace member `{importer}`: {e}"))?)
	}
}

impl WorkspacePackageSource {
	fn inspect(manifest: &Manifest) -> Result<InspectResult, SourceError> {
		let structure = PackageStructure::Pesde(
			manifest
				.exports
				.clone()
				.ok_or("workspace member manifest missing exports")?,
			manifest.config.clone(),
		);

		Ok(InspectResult {
			structure,
			dependencies: manifest.dependencies.clone().into_inner(),
		})
	}
}
