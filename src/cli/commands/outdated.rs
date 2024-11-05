use std::collections::HashSet;

use anyhow::Context;
use clap::Args;
use semver::VersionReq;

use pesde::{
    source::{
        specifiers::DependencySpecifiers,
        traits::{PackageRef, PackageSource},
    },
    Project,
};

#[derive(Debug, Args)]
pub struct OutdatedCommand {
    /// Whether to check within version requirements
    #[arg(short, long)]
    strict: bool,
}

impl OutdatedCommand {
    pub async fn run(self, project: Project) -> anyhow::Result<()> {
        let graph = project.deser_lockfile().await?.graph;

        let manifest = project
            .deser_manifest()
            .await
            .context("failed to read manifest")?;

        let mut refreshed_sources = HashSet::new();

        for (name, versions) in graph {
            for (current_version_id, node) in versions {
                let Some((alias, mut specifier)) = node.node.direct else {
                    continue;
                };

                if matches!(
                    specifier,
                    DependencySpecifiers::Git(_) | DependencySpecifiers::Workspace(_)
                ) {
                    continue;
                }

                let source = node.node.pkg_ref.source();

                if refreshed_sources.insert(source.clone()) {
                    source.refresh(&project).await?;
                }

                if !self.strict {
                    match specifier {
                        DependencySpecifiers::Pesde(ref mut spec) => {
                            spec.version = VersionReq::STAR;
                        }
                        #[cfg(feature = "wally-compat")]
                        DependencySpecifiers::Wally(ref mut spec) => {
                            spec.version = VersionReq::STAR;
                        }
                        DependencySpecifiers::Git(_) => {}
                        DependencySpecifiers::Workspace(_) => {}
                    };
                }

                let version_id = source
                    .resolve(&specifier, &project, manifest.target.kind())
                    .await
                    .context("failed to resolve package versions")?
                    .1
                    .pop_last()
                    .map(|(v_id, _)| v_id)
                    .context(format!("no versions of {specifier} found"))?;

                if version_id != current_version_id {
                    println!("{name} ({alias}) {current_version_id} -> {version_id}");
                }
            }
        }

        Ok(())
    }
}
