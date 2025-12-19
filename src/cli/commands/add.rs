use std::str::FromStr as _;

use anyhow::Context as _;
use clap::Args;
use semver::VersionReq;

use crate::cli::{
	AnyPackageIdentifier, VersionedPackageName, config::read_config, dep_type_to_key,
};
use pesde::{
	DEFAULT_INDEX_NAME, Project, RefreshedSources,
	manifest::{Alias, DependencyType, target::TargetKind},
	names::PackageNames,
	source::{
		PackageSources,
		git::{
			GitPackageSource,
			specifier::{GitDependencySpecifier, GitVersionSpecifier},
		},
		path::{PathPackageSource, specifier::PathDependencySpecifier},
		pesde::{PesdePackageSource, specifier::PesdeDependencySpecifier},
		specifiers::DependencySpecifiers,
		traits::{PackageSource as _, RefreshOptions, ResolveOptions},
		workspace::{WorkspacePackageSource, specifier::WorkspaceDependencySpecifier},
	},
};

#[derive(Debug, Args)]
pub struct AddCommand {
	/// The package name to add
	#[arg(index = 1)]
	name: AnyPackageIdentifier<VersionReq>,

	/// The index in which to search for the package
	#[arg(short, long)]
	index: Option<String>,

	/// The target environment of the package
	#[arg(short, long)]
	target: Option<TargetKind>,

	/// The alias to use for the package
	#[arg(short, long)]
	alias: Option<Alias>,

	/// Whether to add the package as a peer dependency
	#[arg(short, long)]
	peer: bool,

	/// Whether to add the package as a dev dependency
	#[arg(short, long, conflicts_with = "peer")]
	dev: bool,
}

impl AddCommand {
	pub async fn run(self, project: Project) -> anyhow::Result<()> {
		let manifest = project
			.deser_manifest()
			.await
			.context("failed to read manifest")?;

		let (source, specifier) = match &self.name {
			AnyPackageIdentifier::PackageName(versioned) => match &versioned {
				VersionedPackageName(PackageNames::Pesde(name), version) => {
					let index = manifest
						.indices
						.get(self.index.as_deref().unwrap_or(DEFAULT_INDEX_NAME))
						.cloned();

					if let Some(index) = self.index.as_ref().filter(|_| index.is_none()) {
						anyhow::bail!("index {index} not found");
					}

					let index = match index {
						Some(index) => index,
						None => read_config().await?.default_index,
					};

					let source = PackageSources::Pesde(PesdePackageSource::new(index));
					let specifier = DependencySpecifiers::Pesde(PesdeDependencySpecifier {
						name: name.clone(),
						version: version.clone().unwrap_or(VersionReq::STAR),
						index: self.index.unwrap_or_else(|| DEFAULT_INDEX_NAME.to_string()),
						target: self.target,
					});

					(source, specifier)
				}
				#[cfg(feature = "wally-compat")]
				VersionedPackageName(PackageNames::Wally(name), version) => {
					let index = manifest
						.wally_indices
						.get(self.index.as_deref().unwrap_or(DEFAULT_INDEX_NAME))
						.cloned();

					if let Some(index) = self.index.as_ref().filter(|_| index.is_none()) {
						anyhow::bail!("wally index {index} not found");
					}

					let index = index.context("no wally index found")?;

					let source =
						PackageSources::Wally(pesde::source::wally::WallyPackageSource::new(index));
					let specifier = DependencySpecifiers::Wally(
						pesde::source::wally::specifier::WallyDependencySpecifier {
							name: name.clone(),
							version: version.clone().unwrap_or(VersionReq::STAR),
							index: self.index.unwrap_or_else(|| DEFAULT_INDEX_NAME.to_string()),
						},
					);

					(source, specifier)
				}
			},
			AnyPackageIdentifier::Url((url, rev)) => (
				PackageSources::Git(GitPackageSource::new(url.clone())),
				DependencySpecifiers::Git(GitDependencySpecifier {
					repo: url.clone(),
					version_specifier: GitVersionSpecifier::Rev(rev.clone()),
					path: None,
				}),
			),
			AnyPackageIdentifier::Workspace(VersionedPackageName(name, version)) => (
				PackageSources::Workspace(WorkspacePackageSource),
				DependencySpecifiers::Workspace(WorkspaceDependencySpecifier {
					name: name.clone(),
					version: version.clone().unwrap_or_default(),
					target: self.target,
				}),
			),
			AnyPackageIdentifier::Path(path) => (
				PackageSources::Path(PathPackageSource),
				DependencySpecifiers::Path(PathDependencySpecifier { path: path.clone() }),
			),
		};

		let refreshed_sources = RefreshedSources::new();

		refreshed_sources
			.refresh(
				&source,
				&RefreshOptions {
					project: project.clone(),
				},
			)
			.await
			.context("failed to refresh package source")?;

		let (_, mut versions, suggestions) = source
			.resolve(
				&specifier,
				&ResolveOptions {
					project: project.clone(),
					target: manifest.target.kind(),
					refreshed_sources,
					loose_target: false,
				},
			)
			.await
			.context("failed to resolve package")?;

		let Some((version_id, _)) = versions.pop_last() else {
			anyhow::bail!(
				"no matching versions found for package{}",
				if suggestions.is_empty() {
					"".into()
				} else {
					format!(
						". available targets: {}",
						suggestions
							.into_iter()
							.map(|t| t.to_string())
							.collect::<Vec<_>>()
							.join(", ")
					)
				}
			);
		};

		let project_target = manifest.target.kind();
		let mut manifest = toml_edit::DocumentMut::from_str(
			&project
				.read_manifest()
				.await
				.context("failed to read manifest")?,
		)
		.context("failed to parse manifest")?;
		let dependency_key = dep_type_to_key(if self.peer {
			DependencyType::Peer
		} else if self.dev {
			DependencyType::Dev
		} else {
			DependencyType::Standard
		});

		let alias = match self.alias {
			Some(alias) => alias,
			None => match &self.name {
				AnyPackageIdentifier::PackageName(versioned) => versioned.0.name().to_string(),
				AnyPackageIdentifier::Url((url, _)) => url
					.as_url()
					.path
					.to_string()
					.split('/')
					.next_back()
					.map_or_else(|| url.as_url().path.to_string(), ToString::to_string),
				AnyPackageIdentifier::Workspace(versioned) => versioned.0.name().to_string(),
				AnyPackageIdentifier::Path(path) => path
					.file_name()
					.map(|s| s.to_string_lossy().to_string())
					.expect("path has no file name"),
			}
			.parse()
			.context("auto-generated alias is invalid. use --alias to specify one")?,
		};

		let field = &mut manifest[dependency_key]
			.or_insert(toml_edit::Item::Table(toml_edit::Table::new()))[alias.as_str()];

		match specifier {
			DependencySpecifiers::Pesde(spec) => {
				field["name"] = toml_edit::value(spec.name.to_string());
				field["version"] = toml_edit::value(format!("^{}", version_id.version()));

				if version_id.target() != project_target {
					field["target"] = toml_edit::value(version_id.target().to_string());
				}

				if spec.index != DEFAULT_INDEX_NAME {
					field["index"] = toml_edit::value(spec.index);
				}

				println!(
					"added {}@{} {} to {dependency_key}",
					spec.name,
					version_id.version(),
					version_id.target()
				);
			}
			#[cfg(feature = "wally-compat")]
			DependencySpecifiers::Wally(spec) => {
				let name_str = spec.name.to_string();
				let name_str = name_str.trim_start_matches("wally#");
				field["wally"] = toml_edit::value(name_str);
				field["version"] = toml_edit::value(format!("^{}", version_id.version()));

				if spec.index != DEFAULT_INDEX_NAME {
					field["index"] = toml_edit::value(spec.index);
				}

				println!(
					"added wally {name_str}@{} to {dependency_key}",
					version_id.version()
				);
			}
			DependencySpecifiers::Git(spec) => {
				field["repo"] = toml_edit::value(spec.repo.to_string());
				match spec.version_specifier.clone() {
					GitVersionSpecifier::Rev(rev) => field["rev"] = toml_edit::value(rev),
					GitVersionSpecifier::VersionReq(req) => {
						field["version"] = toml_edit::value(req.to_string());
					}
				}

				println!(
					"added git {}{} to {dependency_key}",
					spec.repo, spec.version_specifier
				);
			}
			DependencySpecifiers::Workspace(spec) => {
				field["workspace"] = toml_edit::value(spec.name.to_string());
				if let AnyPackageIdentifier::Workspace(versioned) = self.name
					&& let Some(version) = versioned.1
				{
					field["version"] = toml_edit::value(version.to_string());
				}

				println!(
					"added workspace {}@{} to {dependency_key}",
					spec.name, spec.version
				);
			}
			DependencySpecifiers::Path(spec) => {
				field["path"] = toml_edit::value(spec.path.to_string_lossy().to_string());

				println!("added path {} to {dependency_key}", spec.path.display());
			}
		}

		project
			.write_manifest(manifest.to_string())
			.await
			.context("failed to write manifest")?;

		Ok(())
	}
}
