use crate::cli::{download_graph, repos::update_scripts, run_on_workspace_members};
use anyhow::Context;
use clap::Args;
use colored::Colorize;
use indicatif::MultiProgress;
use pesde::{lockfile::Lockfile, Project};
use std::collections::HashSet;

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
            .dependency_graph(None, &mut refreshed_sources)
            .await
            .context("failed to build dependency graph")?;

        update_scripts(&project).await?;

        project
            .write_lockfile(Lockfile {
                name: manifest.name,
                version: manifest.version,
                target: manifest.target.kind(),
                overrides: manifest.overrides,

                graph: download_graph(
                    &project,
                    &mut refreshed_sources,
                    &graph,
                    &multi,
                    &reqwest,
                    false,
                    false,
                    "📥 downloading dependencies".to_string(),
                    "📥 downloaded dependencies".to_string(),
                )
                .await?,

                workspace: run_on_workspace_members(&project, |project| {
                    let multi = multi.clone();
                    let reqwest = reqwest.clone();
                    async move { Box::pin(self.run(project, multi, reqwest)).await }
                })
                .await?,
            })
            .await
            .context("failed to write lockfile")?;

        Ok(())
    }
}
