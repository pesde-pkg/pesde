use anyhow::Context as _;
use clap::Args;
use clap::Subcommand;
use pesde::registry::*;

use crate::cli::registry::TreeUpdate;
use crate::cli::ui::command::registry::scope::ScopeCommandContext;
use crate::cli::ui::print_immutable_protocol_warning;
use crate::cli::ui::public_key::UserInputPublicKey;
use crate::cli::ui::signature::SignatureSourceArgs;

/// Rotates (updates) your key in a scope
#[derive(Debug, Args)]
pub struct RotateKeyCommand {
	/// Whether to sign the operation without prompting for confirmation
	#[arg(long)]
	no_prompt: bool,

	#[command(subcommand)]
	from: From,
}

#[derive(Debug, Subcommand)]
enum From {
	/// The public key to rotate from
	From {
		/// The public key to rotate from
		public_key: UserInputPublicKey,

		#[command(flatten)]
		source: SignatureSourceArgs,

		#[command(subcommand)]
		to: To,
	},
}

#[derive(Debug, Subcommand)]
enum To {
	/// The public key to rotate to
	To {
		/// The public key to rotate to
		public_key: UserInputPublicKey,

		#[command(flatten)]
		source: SignatureSourceArgs,
	},
}

impl RotateKeyCommand {
	pub async fn run(self, ctx: ScopeCommandContext) -> anyhow::Result<()> {
		let From::From {
			public_key: UserInputPublicKey {
				key: from_key,
				was_external_format: _,
			},
			to,
			source: from_source,
		} = self.from;
		let To::To {
			public_key: UserInputPublicKey {
				key: to_key,
				was_external_format: _,
			},
			source: to_source,
		} = to;

		if from_key == to_key {
			anyhow::bail!("the 'from' and 'to' keys must be different");
		}

		eprintln!("scope: {}", ctx.scope());
		eprintln!("from public key: {from_key}");
		eprintln!("to public key: {to_key}");
		eprintln!(
			"WARNING: you are about to push an operation to the registry that will rotate the key for this scope"
		);

		eprintln!();

		print_immutable_protocol_warning();

		eprintln!();

		if !self.no_prompt {
			inquire::Confirm::new("do you want to continue?")
				.with_default(false)
				.with_help_message("this action cannot be undone")
				.prompt()
				.context("failed to read user input")?
				.ok_or_else(|| anyhow::anyhow!("aborted by user"))?;
		}

		let client = ctx.client();

		let scope_id = ScopeId::from(ctx.scope());

		let tip_entry = client
			.scope_head(&scope_id)
			.await?
			.context("scope does not exist")?
			.tip
			.entry;

		let mut scope_state = tip_entry.payload.header().post_state.clone();
		if scope_state.owner == from_key {
			scope_state.owner = to_key.clone();
		} else {
			let scope_member = ScopeMember(from_key.clone());
			let scope_member_value = client
				.tree_entry(&scope_state.scope_members_root, &scope_member)
				.await?
				.with_context(|| format!("scope member {from_key} not found"))?;

			scope_state.scope_members_root = client
				.tree_transition(
					&scope_state.scope_members_root,
					[
						TreeUpdate::Remove { key: scope_member },
						TreeUpdate::Insert {
							key: ScopeMember(to_key.clone()),
							value: scope_member_value,
						},
					],
				)
				.await?;
		}

		let consent = to_source
			.consent::<RotateKeyUserScopeOpBody>(&ctx, &scope_id, to_key)
			.await?;

		let body = RotateKeyUserScopeOpBody::new(
			UserScopeOpHeader {
				common: ScopeOpHeader {
					scope_id,
					prev_hash: tip_entry.hash(),
					post_state: scope_state,
				},
				signer: from_key,
			},
			consent.into_inner(),
		)
		.context("failed to construct rotate key operation")?;

		let body = from_source
			.signed(&ctx, UserScopeOp::RotateKey(body))
			.await?;
		let body = ScopeEntryPayload::User(body);

		let response = client.post_scope_entry(&body).await?;

		println!(
			"successfully rotated key for scope. server operation timestamp: {}",
			response.published_at
		);

		Ok(())
	}
}
