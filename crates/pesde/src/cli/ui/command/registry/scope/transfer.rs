use anyhow::Context as _;
use clap::Args;
use pesde::registry::*;

use crate::cli::registry::TreeUpdate;
use crate::cli::ui::command::registry::scope::ScopeCommandContext;
use crate::cli::ui::signature::SignatureSourceArgs;

/// Transfers the ownership of a scope to another identity
#[derive(Debug, Args)]
pub struct TransferCommand {
	/// The consent message produced by the candidate owner of the scope
	message: Box<str>,

	#[command(flatten)]
	source: SignatureSourceArgs,
}

impl TransferCommand {
	pub async fn run(self, ctx: ScopeCommandContext) -> anyhow::Result<()> {
		let consent = fast32::base64::RFC4648_NOPAD
			.decode_str(&self.message)
			.context("malformed message")?;

		let consent: UnvalidatedSigned<UnvalidatedConsent> =
			serde_json::from_slice(&consent).context("failed to parse message")?;

		let scope_id = ScopeId::from(ctx.scope());

		let consent = Consent::<TransferOwnershipUserScopeOpBody>::new(&scope_id, consent)
			// check to avoid sending redundant requests
			.context("consent was not for the expected scope or had a malformed signature")?
			.into_inner();

		eprintln!("scope: {}", ctx.scope());
		eprintln!("candidate public key: {}", consent.body.consenter);
		eprintln!(
			"candidate nonce: {}",
			fast32::base32::CROCKFORD_LOWER.encode(&consent.body.nonce)
		);

		eprintln!(
			"WARNING: you are about to push an operation to the registry that will remove your ties to the scope and make the candidate the new owner"
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

		let old_owner = std::mem::replace(&mut scope_state.owner, consent.body.consenter.clone());

		scope_state.scope_members_root = client
			.tree_transition(
				&scope_state.scope_members_root,
				[TreeUpdate::RemoveIfPresent {
					key: ScopeMember(consent.body.consenter.clone()),
				}],
			)
			.await?;

		let body = TransferOwnershipUserScopeOpBody::new(
			UserScopeOpHeader {
				signer: old_owner,
				common: ScopeOpHeader {
					scope_id,
					prev_hash: tip_entry.hash(),
					post_state: scope_state,
				},
			},
			consent,
		)
		.context("failed to construct transfer ownership operation")?;

		let body = self
			.source
			.signed(&ctx, UserScopeOp::TransferOwnership(body))
			.await?;
		let body = ScopeEntryPayload::User(body);

		let response = client.post_scope_entry(&body).await?;

		println!(
			"successfully transferred scope ownership. server operation timestamp: {}",
			response.published_at
		);

		Ok(())
	}
}
