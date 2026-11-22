use std::ops::Deref;
use std::str::FromStr as _;
use std::sync::Arc;

use anyhow::Context as _;
use clap::Args;
use clap::Subcommand;
use pesde::Url;

use crate::cli::registry::RegistryClient;
use crate::cli::ui::CommandContext;

/// Registry related commands
#[derive(Debug, Args)]
pub struct RegistryCommand {
	/// The source name (only pesde) or registry URL to work with. If not specified, the default registry will be used
	#[arg(short, long)]
	registry: Option<Box<str>>,

	#[command(subcommand)]
	subcommand: RegistrySubcommand,
}

mod publish;
mod scope;

#[derive(Debug, Subcommand)]
enum RegistrySubcommand {
	Scope(scope::ScopeCommand),
	Publish(publish::PublishCommand),
}

impl RegistryCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let registry = if let Some(registry) = self.registry {
			if let Ok(url) = Url::from_str(&registry) {
				url
			} else {
				let manifest = ctx.member().await?.deser_manifest().await?;
				manifest
					.registry_named(&registry)
					.cloned()
					.with_context(|| format!("registry `{registry}` not found in manifest"))?
					.into_inner()
			}
		} else {
			ctx.cli_config().await?.default_registry.clone()
		};

		let ctx = RegistryCommandContext {
			client: ctx.registries().await?.client(registry).await?,
			cmd: ctx,
		};

		match self.subcommand {
			RegistrySubcommand::Scope(cmd) => cmd.run(ctx).await,
			RegistrySubcommand::Publish(cmd) => cmd.run(ctx).await,
		}
	}
}

pub struct RegistryCommandContext {
	cmd: CommandContext,
	client: Arc<RegistryClient>,
}

impl Deref for RegistryCommandContext {
	type Target = CommandContext;

	fn deref(&self) -> &Self::Target {
		&self.cmd
	}
}

impl RegistryCommandContext {
	pub fn client(&self) -> &RegistryClient {
		&self.client
	}
}
