use crate::cli::{progress_bar, repos::update_scripts, run_on_workspace_members};
use anyhow::Context;
use clap::Args;
use colored::Colorize;
use indicatif::MultiProgress;
use pesde::{lockfile::Lockfile, Project};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::Mutex;

#[derive(Debug, Args, Copy, Clone)]
pub struct UpdateCommand {}

impl UpdateCommand {
    pub async fn run(
        self,
        project: Project,
        multi: MultiProgress,
        reqwest: reqwest::Client,
    ) -> anyhow::Result<()> {
        let mut refreshed_sources = HashSet::new();

        let manifest = project
            .deser_manifest()
            .await
            .context("failed to read manifest")?;

        println!(
            "\n{}\n",
            format!("[now updating {} {}]", manifest.name, manifest.target)
                .bold()
                .on_bright_black()
        );

        let graph = project
            .dependency_graph(None, &mut refreshed_sources, false)
            .await
            .context("failed to build dependency graph")?;
        let graph = Arc::new(graph);

        update_scripts(&project).await?;

        project
            .write_lockfile(Lockfile {
                name: manifest.name,
                version: manifest.version,
                target: manifest.target.kind(),
                overrides: manifest.overrides,

                graph: {
                    let (rx, downloaded_graph) = project
                        .download_and_link(
                            &graph,
                            &Arc::new(Mutex::new(refreshed_sources)),
                            &reqwest,
                            false,
                            false,
                            |_| async { Ok::<_, std::io::Error>(()) },
                        )
                        .await
                        .context("failed to download dependencies")?;

                    progress_bar(
                        graph.values().map(|versions| versions.len() as u64).sum(),
                        rx,
                        &multi,
                        "📥 ".to_string(),
                        "downloading dependencies".to_string(),
                        "downloaded dependencies".to_string(),
                    )
                    .await?;

                    downloaded_graph
                        .await
                        .context("failed to download dependencies")?
                },

                workspace: run_on_workspace_members(&project, |project| {
                    let multi = multi.clone();
                    let reqwest = reqwest.clone();
                    async move { Box::pin(self.run(project, multi, reqwest)).await }
                })
                .await?,
            })
            .await
            .context("failed to write lockfile")?;

        println!(
            "\n\n{}. run `{} install` in order to install the new dependencies",
            "✅ done".green(),
            env!("CARGO_BIN_NAME")
        );

        Ok(())
    }
}
