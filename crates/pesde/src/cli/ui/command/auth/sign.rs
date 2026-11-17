use bstr::BString;
use clap::Args;
use tokio::io::AsyncReadExt as _;

use crate::cli::config::read_config;
use crate::cli::ui::public_key::UserInputPublicKey;
use crate::cli::ui::signature::SignatureSourceArgs;

/// Signs a message with a key and its associated signature source, printing the signature to stdout
#[derive(Debug, Args)]
pub struct SignCommand {
	/// The key to sign the message with
	key: UserInputPublicKey,

	/// The message to sign. If '-', the message is read from stdin
	#[arg(default_value = "-")]
	message: Box<str>,

	#[command(flatten)]
	source: SignatureSourceArgs,
}

impl SignCommand {
	pub async fn run(self) -> anyhow::Result<()> {
		let source = self
			.source
			.get_source_from_config(&self.key.key, async |key| {
				Ok(read_config().await?.signature_sources.get(key).copied())
			})
			.await?;

		let message = if &*self.message == "-" {
			let mut buffer = Vec::new();
			tokio::io::stdin().read_to_end(&mut buffer).await?;
			buffer
		} else {
			self.message.into_boxed_bytes().to_vec()
		};

		let signature = source.sign(&self.key.key, &message).await?;

		eprintln!(
			"signature for `{}` with `{}`:",
			BString::new(message),
			self.key.key
		);
		print!("{signature}");

		Ok(())
	}
}
