use std::str::FromStr;

use anyhow::Context;
use clap::Args;
use semver::VersionReq;

use crate::cli::{config::read_config, AnyPackageIdentifier, VersionedPackageName};
use pesde::{
    manifest::target::TargetKind,
    names::PackageNames,
    source::{
        git::{specifier::GitDependencySpecifier, GitPackageSource},
        pesde::{specifier::PesdeDependencySpecifier, PesdePackageSource},
        specifiers::DependencySpecifiers,
        traits::PackageSource,
        workspace::WorkspacePackageSource,
        PackageSources,
    },
    Project, DEFAULT_INDEX_NAME,
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
    alias: Option<String>,

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
                        log::error!("index {index} not found");
                        return Ok(());
                    }

                    let index = match index {
                        Some(index) => index,
                        None => read_config().await?.default_index,
                    };

                    let source = PackageSources::Pesde(PesdePackageSource::new(index));
                    let specifier = DependencySpecifiers::Pesde(PesdeDependencySpecifier {
                        name: name.clone(),
                        version: version.clone().unwrap_or(VersionReq::STAR),
                        index: self.index,
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
                        log::error!("wally index {index} not found");
                        return Ok(());
                    }

                    let index = index.context("no wally index found")?;

                    let source =
                        PackageSources::Wally(pesde::source::wally::WallyPackageSource::new(index));
                    let specifier = DependencySpecifiers::Wally(
                        pesde::source::wally::specifier::WallyDependencySpecifier {
                            name: name.clone(),
                            version: version.clone().unwrap_or(VersionReq::STAR),
                            index: self.index,
                        },
                    );

                    (source, specifier)
                }
            },
            AnyPackageIdentifier::Url((url, rev)) => (
                PackageSources::Git(GitPackageSource::new(url.clone())),
                DependencySpecifiers::Git(GitDependencySpecifier {
                    repo: url.clone(),
                    rev: rev.to_string(),
                    path: None,
                }),
            ),
            AnyPackageIdentifier::Workspace(VersionedPackageName(name, version)) => (
                PackageSources::Workspace(WorkspacePackageSource),
                DependencySpecifiers::Workspace(
                    pesde::source::workspace::specifier::WorkspaceDependencySpecifier {
                        name: name.clone(),
                        version: version.clone().unwrap_or_default(),
                        target: self.target,
                    },
                ),
            ),
        };
        source
            .refresh(&project)
            .await
            .context("failed to refresh package source")?;

        let Some(version_id) = source
            .resolve(&specifier, &project, manifest.target.kind())
            .await
            .context("failed to resolve package")?
            .1
            .pop_last()
            .map(|(v_id, _)| v_id)
        else {
            log::error!("no versions found for package {specifier}");

            return Ok(());
        };

        let project_target = manifest.target.kind();
        let mut manifest = toml_edit::DocumentMut::from_str(
            &project
                .read_manifest()
                .await
                .context("failed to read manifest")?,
        )
        .context("failed to parse manifest")?;
        let dependency_key = if self.peer {
            "peer_dependencies"
        } else if self.dev {
            "dev_dependencies"
        } else {
            "dependencies"
        };

        let alias = self.alias.unwrap_or_else(|| match self.name.clone() {
            AnyPackageIdentifier::PackageName(versioned) => versioned.0.as_str().1.to_string(),
            AnyPackageIdentifier::Url((url, _)) => url
                .path
                .to_string()
                .split('/')
                .last()
                .map(|s| s.to_string())
                .unwrap_or(url.path.to_string()),
            AnyPackageIdentifier::Workspace(versioned) => versioned.0.as_str().1.to_string(),
        });

        let field = &mut manifest[dependency_key]
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))[&alias];

        match specifier {
            DependencySpecifiers::Pesde(spec) => {
                field["name"] = toml_edit::value(spec.name.clone().to_string());
                field["version"] = toml_edit::value(format!("^{}", version_id.version()));

                if *version_id.target() != project_target {
                    field["target"] = toml_edit::value(version_id.target().to_string());
                }

                if let Some(index) = spec.index.filter(|i| i != DEFAULT_INDEX_NAME) {
                    field["index"] = toml_edit::value(index);
                }

                println!(
                    "added {}@{} {} to {}",
                    spec.name,
                    version_id.version(),
                    version_id.target(),
                    dependency_key
                );
            }
            #[cfg(feature = "wally-compat")]
            DependencySpecifiers::Wally(spec) => {
                field["wally"] = toml_edit::value(spec.name.clone().to_string());
                field["version"] = toml_edit::value(format!("^{}", version_id.version()));

                if let Some(index) = spec.index.filter(|i| i != DEFAULT_INDEX_NAME) {
                    field["index"] = toml_edit::value(index);
                }

                println!(
                    "added wally {}@{} to {}",
                    spec.name,
                    version_id.version(),
                    dependency_key
                );
            }
            DependencySpecifiers::Git(spec) => {
                field["repo"] = toml_edit::value(spec.repo.to_bstring().to_string());
                field["rev"] = toml_edit::value(spec.rev.clone());

                println!("added git {}#{} to {}", spec.repo, spec.rev, dependency_key);
            }
            DependencySpecifiers::Workspace(spec) => {
                field["workspace"] = toml_edit::value(spec.name.clone().to_string());
                if let AnyPackageIdentifier::Workspace(versioned) = self.name {
                    if let Some(version) = versioned.1 {
                        field["version"] = toml_edit::value(version.to_string());
                    }
                }

                println!(
                    "added workspace {}@{} to {}",
                    spec.name, spec.version, dependency_key
                );
            }
        }

        project
            .write_manifest(manifest.to_string())
            .await
            .context("failed to write manifest")?;

        Ok(())
    }
}
