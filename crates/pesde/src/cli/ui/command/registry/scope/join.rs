use clap::Args;
use pesde::registry::AddMemberUserScopeOpBody;
use pesde::registry::ScopeId;

use crate::cli::ui::command::registry::scope::ScopeCommandContext;
use crate::cli::ui::public_key::UserInputPublicKey;
use crate::cli::ui::signature::SignatureSourceArgs;

/// Creates a message that will allow the key to join the scope
#[derive(Debug, Args)]
pub struct JoinCommand {
	/// The public key to add as a member of the scope
	public_key: UserInputPublicKey,

	#[command(flatten)]
	source: SignatureSourceArgs,
}

impl JoinCommand {
	pub async fn run(self, ctx: ScopeCommandContext) -> anyhow::Result<()> {
		eprintln!(
			"WARNING: you are about to sign a message that will allow the owner of the scope to add you as a member"
		);
		eprintln!("you may be liable for any actions taken by the scope's management");

		eprintln!();

		ctx.signature_lecture()?;

		let scope_id = ScopeId::from(ctx.scope());

		let consent = self
			.source
			.consent::<AddMemberUserScopeOpBody>(&ctx, &scope_id, self.public_key.key)
			.await?;

		eprintln!("send this message to the owner of the scope to request membership:");

		print!(
			"{}",
			fast32::base64::RFC4648_NOPAD.encode(&serde_json::to_vec(&consent)?)
		);

		Ok(())
	}
}
