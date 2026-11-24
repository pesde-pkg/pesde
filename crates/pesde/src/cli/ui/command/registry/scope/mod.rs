use clap::Args;
use clap::Subcommand;
use pesde::registry::Scope;
use std::ops::Deref;

use crate::cli::ui::command::registry::RegistryCommandContext;

mod add_member;
mod join;
mod rotate_key;
mod takeover;
mod transfer;

/// Scope related commands
#[derive(Debug, Args)]
pub struct ScopeCommand {
	/// The scope to work with
	scope: Scope,

	/// The subcommand to run
	#[command(subcommand)]
	subcommand: ScopeSubcommand,
}

#[derive(Debug, Subcommand)]
enum ScopeSubcommand {
	Join(join::JoinCommand),

	AddMember(add_member::AddMemberCommand),

	Takeover(takeover::TakeoverCommand),

	Transfer(transfer::TransferCommand),

	RotateKey(rotate_key::RotateKeyCommand),
}

impl ScopeCommand {
	pub async fn run(self, ctx: RegistryCommandContext) -> anyhow::Result<()> {
		let ctx = ScopeCommandContext {
			registry: ctx,
			scope: self.scope,
		};

		match self.subcommand {
			ScopeSubcommand::Join(cmd) => cmd.run(ctx).await,
			ScopeSubcommand::AddMember(cmd) => cmd.run(ctx).await,
			ScopeSubcommand::Takeover(cmd) => cmd.run(ctx).await,
			ScopeSubcommand::Transfer(cmd) => cmd.run(ctx).await,
			ScopeSubcommand::RotateKey(cmd) => cmd.run(ctx).await,
		}
	}
}

pub struct ScopeCommandContext {
	registry: RegistryCommandContext,
	scope: Scope,
}

impl ScopeCommandContext {
	pub fn scope(&self) -> &Scope {
		&self.scope
	}
}

impl Deref for ScopeCommandContext {
	type Target = RegistryCommandContext;

	fn deref(&self) -> &Self::Target {
		&self.registry
	}
}
