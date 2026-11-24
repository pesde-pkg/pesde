use anyhow::Context as _;
use clap::Args;
use pesde::registry::*;

use crate::cli::registry::TreeUpdate;
use crate::cli::ui::command::registry::scope::ScopeCommandContext;
use crate::cli::ui::public_key::UserInputPublicKey;
use crate::cli::ui::scope_grant::ScopeGrantArgs;
use crate::cli::ui::signature::SignatureSourceArgs;

/// Updates a member's grant in a scope
#[derive(Debug, Args)]
pub struct UpdateMemberGrantCommand {
	/// The member to update the grant of
	member: UserInputPublicKey,

	#[command(flatten)]
	grant: ScopeGrantArgs,

	#[command(flatten)]
	source: SignatureSourceArgs,
}

impl UpdateMemberGrantCommand {
	pub async fn run(self, ctx: ScopeCommandContext) -> anyhow::Result<()> {
		let grant = self.grant.get()?;

		eprintln!(
			"WARNING: you are about to push an operation to the registry that will update a member's grant in the scope"
		);

		eprintln!();

		ctx.signature_lecture()?;

		let scope_id = ScopeId::from(ctx.scope());

		let client = ctx.client();

		// TODO: check transparency log consistency & inclusion proofs
		let tip_entry = client
			.scope_head(&scope_id)
			.await?
			.context("scope does not exist")?
			.tip
			.entry;

		let mut scope_state = tip_entry.payload.header().post_state.clone();

		let key = ScopeMember(self.member.key);

		scope_state.scope_members_root = client
			.tree_transition(
				&scope_state.scope_members_root,
				[TreeUpdate::Update {
					key: key.clone(),
					value: ScopeMemberValue {
						grant: grant.clone(),
					},
				}],
			)
			.await?;

		let body = UpdateMemberGrantUserScopeOpBody {
			header: UserScopeOpHeader {
				signer: scope_state.owner.clone(),
				common: ScopeOpHeader {
					scope_id,
					prev_hash: tip_entry.hash(),
					post_state: scope_state,
				},
			},
			member: key,
			grant,
		};

		let body = self
			.source
			.signed(&ctx, UserScopeOp::UpdateMemberGrant(body))
			.await?;
		let body = ScopeEntryPayload::User(body);

		let response = client.post_scope_entry(&body).await?;

		println!(
			"successfully updated member grant. server operation timestamp: {}",
			response.published_at
		);

		Ok(())
	}
}
