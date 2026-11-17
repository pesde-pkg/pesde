use std::ops::Deref;
use std::str::FromStr as _;

use anyhow::Context as _;
use clap::Args;
use clap::Subcommand;
use pesde::Url;
use pesde::registry::Scope;
use tokio::sync::OnceCell;

use crate::cli::auth::authorization;
use crate::cli::ui::CommandContext;

mod add_member;

/// Scope related commands
#[derive(Debug, Args)]
pub struct ScopeCommand {
	/// The source name (only pesde) or registry URL to work with. If not specified, the default registry will be used
	#[arg(short, long)]
	registry: Option<Box<str>>,

	/// The scope to work with
	scope: Scope,

	/// The subcommand to run
	#[command(subcommand)]
	subcommand: ScopeSubcommand,
}

#[derive(Debug, Subcommand)]
enum ScopeSubcommand {
	#[command(subcommand)]
	AddMember(add_member::AddMemberCommand),
}

impl ScopeCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let registry = if let Some(registry) = self.registry {
			if let Ok(url) = Url::from_str(&registry) {
				url
			} else {
				let manifest = ctx.member().await?.deser_manifest().await?;
				manifest
					.registry_named(&registry)
					.cloned()
					.with_context(|| format!("registry of `{registry}` not found in manifest"))?
					.into_inner()
			}
		} else {
			ctx.cli_config().await?.default_registry.clone()
		};

		let ctx = ScopeCommandContext {
			cmd: ctx,
			registry,
			scope: self.scope,
			authorization: Default::default(),
		};

		match self.subcommand {
			ScopeSubcommand::AddMember(cmd) => cmd.run(ctx).await,
		}
	}
}

pub struct ScopeCommandContext {
	cmd: CommandContext,
	registry: Url,
	scope: Scope,
	authorization: OnceCell<Option<Box<str>>>,
}

impl ScopeCommandContext {
	pub fn registry(&self) -> &Url {
		&self.registry
	}

	pub fn scope(&self) -> &Scope {
		&self.scope
	}

	pub async fn authorization(&self) -> anyhow::Result<Option<&str>> {
		self.authorization
			.get_or_try_init(async || {
				let Some(script) = self.cli_config().await?.auth_scripts.get(self.registry())
				else {
					return Ok(None);
				};

				authorization(script).await
			})
			.await
			.map(Option::as_deref)
	}
}

impl Deref for ScopeCommandContext {
	type Target = CommandContext;

	fn deref(&self) -> &Self::Target {
		&self.cmd
	}
}
