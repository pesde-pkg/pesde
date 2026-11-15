use bstr::BStr;
use clap::Args;
use pesde::signature::Signature;
use tokio::io::AsyncReadExt as _;

use crate::cli::ui::public_key::UserInputPublicKey;

/// Verifies a signature for a message with a key
#[derive(Debug, Args)]
pub struct VerifyCommand {
	/// The key to verify the signature against
	key: UserInputPublicKey,

	/// The signature to verify
	signature: Signature,

	/// The message that was signed. If '-', the message is read from stdin
	#[arg(default_value = "-")]
	message: Box<str>,
}

impl VerifyCommand {
	pub async fn run(self) -> anyhow::Result<()> {
		let message = if &*self.message == "-" {
			let mut buffer = Vec::new();
			tokio::io::stdin().read_to_end(&mut buffer).await?;
			buffer
		} else {
			self.message.into_boxed_bytes().to_vec()
		};

		eprintln!(
			"signature `{}` for `{}` with `{}`",
			self.signature,
			BStr::new(&message),
			self.key.key
		);

		let valid = self.signature.verify(&self.key.key, &message);
		if !valid {
			anyhow::bail!("invalid");
		}

		eprintln!("valid");

		Ok(())
	}
}
