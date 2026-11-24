use anyhow::Context as _;
use clap::Args;
use pesde::registry::*;

use crate::cli::registry::TreeUpdate;
use crate::cli::ui::command::registry::scope::ScopeCommandContext;
use crate::cli::ui::signature::SignatureSourceArgs;

/// Adds a member to a scope
#[derive(Debug, Args)]
pub struct AddMemberCommand {
	/// The member's consent message
	message: Box<str>,

	#[command(flatten)]
	source: SignatureSourceArgs,
}

impl AddMemberCommand {
	pub async fn run(self, ctx: ScopeCommandContext) -> anyhow::Result<()> {
		let consent = fast32::base64::RFC4648_NOPAD
			.decode_str(&self.message)
			.context("malformed message")?;

		let consent: UnvalidatedSigned<UnvalidatedConsent> =
			serde_json::from_slice(&consent).context("failed to parse message")?;

		let scope_id = ScopeId::from(ctx.scope());

		let consent = Consent::<AddMemberUserScopeOpBody>::new(&scope_id, consent)
			// check to avoid sending redundant requests
			.context("consent was not for the expected scope or had a malformed signature")?
			.into_inner();

		// TODO: prompt for this
		let grant = pesde::registry::ScopeGrant(Default::default());

		eprintln!("scope: {}", ctx.scope());
		eprintln!("member public key: {}", consent.body.consenter);
		eprintln!(
			"member nonce: {}",
			fast32::base32::CROCKFORD_LOWER.encode(&consent.body.nonce)
		);

		eprintln!(
			"WARNING: you are about to push an operation to the registry that will add a member to the scope"
		);

		eprintln!();

		ctx.signature_lecture()?;

		let client = ctx.client();

		// TODO: check transparency log consistency & inclusion proofs
		let tip_entry = client
			.scope_head(&scope_id)
			.await?
			.context("scope does not exist")?
			.tip
			.entry;

		let mut scope_state = tip_entry.payload.header().post_state.clone();

		scope_state.scope_members_root = client
			.tree_transition(
				&scope_state.scope_members_root,
				[TreeUpdate::Insert {
					key: ScopeMember(consent.body.consenter.clone()),
					value: ScopeMemberValue {
						grant: grant.clone(),
					},
				}],
			)
			.await?;

		let body = AddMemberUserScopeOpBody::new(
			UserScopeOpHeader {
				signer: scope_state.owner.clone(),
				common: ScopeOpHeader {
					scope_id,
					prev_hash: tip_entry.hash(),
					post_state: scope_state,
				},
			},
			consent,
			grant,
		)
		.context("failed to construct add member operation")?;

		let body = self
			.source
			.signed(&ctx, UserScopeOp::AddMember(body))
			.await?;
		let body = ScopeEntryPayload::User(body);

		let response = client.post_scope_entry(&body).await?;

		println!(
			"successfully added member to scope. server operation timestamp: {}",
			response.published_at
		);

		Ok(())
	}
}
