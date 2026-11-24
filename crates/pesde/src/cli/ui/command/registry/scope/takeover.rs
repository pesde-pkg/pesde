use clap::Args;
use pesde::registry::ScopeId;
use pesde::registry::TransferOwnershipUserScopeOpBody;

use crate::cli::ui::command::registry::scope::ScopeCommandContext;
use crate::cli::ui::public_key::UserInputPublicKey;
use crate::cli::ui::signature::SignatureSourceArgs;

/// Creates a message that will allow the key to take over ownership of the scope
#[derive(Debug, Args)]
pub struct TakeoverCommand {
	/// The public key to make the new owner of the scope
	public_key: UserInputPublicKey,

	#[command(flatten)]
	source: SignatureSourceArgs,
}

impl TakeoverCommand {
	pub async fn run(self, ctx: ScopeCommandContext) -> anyhow::Result<()> {
		eprintln!(
			"WARNING: you are about to sign a message that will allow the owner of the scope to make you the new owner"
		);
		eprintln!("you may be liable for any actions taken by the scope's management & members");

		eprintln!();

		ctx.signature_lecture()?;

		let scope_id = ScopeId::from(ctx.scope());

		let consent = self
			.source
			.consent::<TransferOwnershipUserScopeOpBody>(&ctx, &scope_id, self.public_key.key)
			.await?;

		eprintln!("send this message to the owner of the scope to request ownership transfer:");

		print!(
			"{}",
			fast32::base64::RFC4648_NOPAD.encode(&serde_json::to_vec(&consent)?)
		);

		Ok(())
	}
}
