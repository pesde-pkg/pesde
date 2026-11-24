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

	/// Whether to sign the operation without prompting for confirmation
	#[arg(long)]
	no_prompt: bool,

	#[command(subcommand)]
	subcommand: RegistrySubcommand,
}

mod deprecate;
mod publish;
mod scope;
mod yank;

#[derive(Debug, Subcommand)]
enum RegistrySubcommand {
	Scope(scope::ScopeCommand),
	Publish(publish::PublishCommand),
	Yank(yank::YankCommand),
	Deprecate(deprecate::DeprecateCommand),
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
			no_prompt: self.no_prompt,
		};

		match self.subcommand {
			RegistrySubcommand::Scope(cmd) => cmd.run(ctx).await,
			RegistrySubcommand::Publish(cmd) => cmd.run(ctx).await,
			RegistrySubcommand::Yank(cmd) => cmd.run(ctx).await,
			RegistrySubcommand::Deprecate(cmd) => cmd.run(ctx).await,
		}
	}
}

pub struct RegistryCommandContext {
	cmd: CommandContext,
	client: Arc<RegistryClient>,
	no_prompt: bool,
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

	pub fn signature_lecture(&self) -> anyhow::Result<()> {
		eprintln!(
			"IMPORTANT: the pesde registry protocol is backed by cryptographic data structures, including transparency logs"
		);
		eprintln!(
			"data removal capabilities are limited. make sure you understand the implications of this before sending any data"
		);
		eprintln!(
			"make sure to use unique identifiers, such as independent keys, so that they cannot be used to track you"
		);
		eprintln!("visit the documentation of the registry protocol for more information");

		eprintln!();

		if !self.no_prompt {
			inquire::Confirm::new("do you want to continue?")
				.with_default(false)
				.with_help_message("this action cannot be undone")
				.prompt()
				.context("failed to read user input")?
				.ok_or_else(|| anyhow::anyhow!("aborted by user"))?;
		}

		Ok(())
	}
}
