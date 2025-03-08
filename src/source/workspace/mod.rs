use crate::{
	deser_manifest,
	manifest::target::Target,
	names::PackageNames,
	reporters::DownloadProgressReporter,
	source::{
		fs::PackageFs,
		ids::VersionId,
		specifiers::DependencySpecifiers,
		traits::{DownloadOptions, GetTargetOptions, PackageSource, ResolveOptions},
		workspace::pkg_ref::WorkspacePackageRef,
		ResolveResult,
	},
};
use futures::StreamExt as _;
use relative_path::RelativePathBuf;
use std::collections::BTreeMap;
use tokio::pin;
use tracing::instrument;

/// The workspace package reference
pub mod pkg_ref;
/// The workspace dependency specifier
pub mod specifier;

/// The workspace package source
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkspacePackageSource;

impl PackageSource for WorkspacePackageSource {
	type Specifier = specifier::WorkspaceDependencySpecifier;
	type Ref = WorkspacePackageRef;
	type RefreshError = errors::RefreshError;
	type ResolveError = errors::ResolveError;
	type DownloadError = errors::DownloadError;
	type GetTargetError = errors::GetTargetError;

	#[instrument(skip_all, level = "debug")]
	async fn resolve(
		&self,
		specifier: &Self::Specifier,
		options: &ResolveOptions,
	) -> Result<ResolveResult<Self::Ref>, Self::ResolveError> {
		let ResolveOptions {
			project,
			target: project_target,
			..
		} = options;

		let (path, manifest) = 'finder: {
			let target = specifier.target.unwrap_or(*project_target);

			let members = project.workspace_members(true).await?;
			pin!(members);

			while let Some((path, manifest)) = members.next().await.transpose()? {
				if manifest.name == specifier.name && manifest.target.kind() == target {
					break 'finder (path, manifest);
				}
			}

			return Err(errors::ResolveError::NoWorkspaceMember(
				specifier.name.to_string(),
				target,
			));
		};

		let manifest_target_kind = manifest.target.kind();
		let pkg_ref = WorkspacePackageRef {
			// workspace_dir is guaranteed to be Some by the workspace_members method
			// strip_prefix is guaranteed to be Some by same method
			// from_path is guaranteed to be Ok because we just stripped the absolute path
			path: RelativePathBuf::from_path(
				path.strip_prefix(project.workspace_dir().unwrap_or(project.package_dir()))
					.unwrap(),
			)
			.unwrap(),
			dependencies: manifest
				.all_dependencies()?
				.into_iter()
				.map(|(alias, (mut spec, ty))| {
					match &mut spec {
						DependencySpecifiers::Pesde(spec) => {
							spec.index = manifest
								.indices
								.get(&spec.index)
								.ok_or_else(|| {
									errors::ResolveError::IndexNotFound(
										spec.index.clone(),
										manifest.name.to_string(),
									)
								})?
								.to_string();
						}
						#[cfg(feature = "wally-compat")]
						DependencySpecifiers::Wally(spec) => {
							spec.index = manifest
								.wally_indices
								.get(&spec.index)
								.ok_or_else(|| {
									errors::ResolveError::IndexNotFound(
										spec.index.clone(),
										manifest.name.to_string(),
									)
								})?
								.to_string();
						}
						DependencySpecifiers::Git(_) => {}
						DependencySpecifiers::Workspace(_) => {}
						DependencySpecifiers::Path(_) => {}
					}

					Ok((alias, (spec, ty)))
				})
				.collect::<Result<_, errors::ResolveError>>()?,
		};

		Ok((
			PackageNames::Pesde(manifest.name),
			BTreeMap::from([(
				VersionId::new(manifest.version, manifest_target_kind),
				pkg_ref,
			)]),
		))
	}

	#[instrument(skip_all, level = "debug")]
	async fn download<R: DownloadProgressReporter>(
		&self,
		pkg_ref: &Self::Ref,
		options: &DownloadOptions<R>,
	) -> Result<PackageFs, Self::DownloadError> {
		let DownloadOptions {
			project, reporter, ..
		} = options;

		let path = pkg_ref
			.path
			.to_path(project.workspace_dir().unwrap_or(project.package_dir()));
		let manifest = deser_manifest(&path).await?;

		reporter.report_done();

		Ok(PackageFs::Copy(path, manifest.target.kind()))
	}

	#[instrument(skip_all, level = "debug")]
	async fn get_target(
		&self,
		pkg_ref: &Self::Ref,
		options: &GetTargetOptions,
	) -> Result<Target, Self::GetTargetError> {
		let GetTargetOptions { project, .. } = options;

		let path = pkg_ref
			.path
			.to_path(project.workspace_dir().unwrap_or(project.package_dir()));
		let manifest = deser_manifest(&path).await?;

		Ok(manifest.target)
	}
}

/// Errors that can occur when using a workspace package source
pub mod errors {
	use crate::manifest::target::TargetKind;
	use thiserror::Error;

	/// Errors that can occur when refreshing the workspace package source
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum RefreshError {}

	/// Errors that can occur when resolving a workspace package
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum ResolveError {
		/// An error occurred reading the workspace members
		#[error("failed to read workspace members")]
		ReadWorkspaceMembers(#[from] crate::errors::WorkspaceMembersError),

		/// No workspace member was found with the given name
		#[error("no workspace member found with name {0} and target {1}")]
		NoWorkspaceMember(String, TargetKind),

		/// An error occurred getting all dependencies
		#[error("failed to get all dependencies")]
		AllDependencies(#[from] crate::manifest::errors::AllDependenciesError),

		/// An index of a member package was not found
		#[error("index {0} not found in member {1}")]
		IndexNotFound(String, String),
	}

	/// Errors that can occur when downloading a workspace package
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum DownloadError {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),
	}

	/// Errors that can occur when getting the target of a workspace package
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum GetTargetError {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),
	}
}
