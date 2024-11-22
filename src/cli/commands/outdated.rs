use std::collections::HashSet;

use anyhow::Context;
use clap::Args;
use futures::future::try_join_all;
use semver::VersionReq;

use crate::cli::up_to_date_lockfile;
use pesde::{
    refresh_sources,
    source::{
        refs::PackageRefs,
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
        let graph = match up_to_date_lockfile(&project).await? {
            Some(file) => file.graph,
            None => {
                anyhow::bail!(
                    "lockfile is out of sync, run `{} install` to update it",
                    env!("CARGO_BIN_NAME")
                );
            }
        };

        let manifest = project
            .deser_manifest()
            .await
            .context("failed to read manifest")?;
        let manifest_target_kind = manifest.target.kind();

        let mut refreshed_sources = HashSet::new();

        refresh_sources(
            &project,
            graph
                .iter()
                .flat_map(|(_, versions)| versions.iter())
                .map(|(_, node)| node.node.pkg_ref.source()),
            &mut refreshed_sources,
        )
        .await?;

        try_join_all(
            graph
                .into_iter()
                .flat_map(|(_, versions)| versions.into_iter())
                .map(|(current_version_id, node)| {
                    let project = project.clone();
                    async move {
                        let Some((alias, mut specifier, _)) = node.node.direct else {
                            return Ok::<(), anyhow::Error>(());
                        };

                        if matches!(
                            specifier,
                            DependencySpecifiers::Git(_) | DependencySpecifiers::Workspace(_)
                        ) {
                            return Ok(());
                        }

                        let source = node.node.pkg_ref.source();

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
                            .resolve(&specifier, &project, manifest_target_kind)
                            .await
                            .context("failed to resolve package versions")?
                            .1
                            .pop_last()
                            .map(|(v_id, _)| v_id)
                            .context(format!("no versions of {specifier} found"))?;

                        if version_id != current_version_id {
                            println!(
                                "{} {} ({alias}) {} -> {}",
                                match node.node.pkg_ref {
                                    PackageRefs::Pesde(pkg_ref) => pkg_ref.name.to_string(),
                                    #[cfg(feature = "wally-compat")]
                                    PackageRefs::Wally(pkg_ref) => pkg_ref.name.to_string(),
                                    _ => unreachable!(),
                                },
                                current_version_id.target(),
                                current_version_id.version(),
                                version_id.version()
                            );
                        }

                        Ok(())
                    }
                }),
        )
        .await?;

        Ok(())
    }
}
