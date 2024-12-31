use crate::cli::up_to_date_lockfile;
use anyhow::Context;
use clap::Args;
use futures::future::try_join_all;
use pesde::{
    source::{
        specifiers::DependencySpecifiers,
        traits::{PackageRef, PackageSource, RefreshOptions, ResolveOptions},
    },
    Project, RefreshedSources,
};
use semver::VersionReq;

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

        let refreshed_sources = RefreshedSources::new();

        if try_join_all(graph.into_iter().map(|(current_id, node)| {
            let project = project.clone();
            let refreshed_sources = refreshed_sources.clone();
            async move {
                let Some((alias, mut specifier, _)) = node.node.direct else {
                    return Ok::<bool, anyhow::Error>(true);
                };

                if matches!(
                    specifier,
                    DependencySpecifiers::Git(_)
                        | DependencySpecifiers::Workspace(_)
                        | DependencySpecifiers::Path(_)
                ) {
                    return Ok(true);
                }

                let source = node.node.pkg_ref.source();
                refreshed_sources
                    .refresh(
                        &source,
                        &RefreshOptions {
                            project: project.clone(),
                        },
                    )
                    .await?;

                if !self.strict {
                    match &mut specifier {
                        DependencySpecifiers::Pesde(spec) => {
                            spec.version = VersionReq::STAR;
                        }
                        #[cfg(feature = "wally-compat")]
                        DependencySpecifiers::Wally(spec) => {
                            spec.version = VersionReq::STAR;
                        }
                        DependencySpecifiers::Git(_) => {}
                        DependencySpecifiers::Workspace(_) => {}
                        DependencySpecifiers::Path(_) => {}
                    };
                }

                let version_id = source
                    .resolve(
                        &specifier,
                        &ResolveOptions {
                            project: project.clone(),
                            target: manifest_target_kind,
                            refreshed_sources: refreshed_sources.clone(),
                        },
                    )
                    .await
                    .context("failed to resolve package versions")?
                    .1
                    .pop_last()
                    .map(|(v_id, _)| v_id)
                    .with_context(|| format!("no versions of {specifier} found"))?;

                if version_id != *current_id.version_id() {
                    println!(
                        "{} ({alias}) {} -> {version_id}",
                        current_id.name(),
                        current_id.version_id(),
                    );

                    return Ok(false);
                }

                Ok(true)
            }
        }))
        .await?
        .into_iter()
        .all(|b| b)
        {
            println!("all packages are up to date");
        }

        Ok(())
    }
}
