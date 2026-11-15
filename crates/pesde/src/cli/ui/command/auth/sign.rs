use anyhow::Context as _;
use bstr::BString;
use clap::Args;
use tokio::io::AsyncReadExt as _;

use crate::cli::config::read_config;
use crate::cli::ui::public_key::UserInputPublicKey;
use crate::cli::ui::signature::SignatureSource;

/// Signs a message with a key and its associated signature source, printing the signature to stdout
#[derive(Debug, Args)]
pub struct SignCommand {
	/// The key to sign the message with
	key: UserInputPublicKey,

	/// The message to sign. If '-', the message is read from stdin
	#[arg(default_value = "-")]
	message: Box<str>,

	/// An override for the signature source to use. If not provided, the signature source associated with the key is used
	#[arg(long)]
	source: Option<SignatureSource>,
}

impl SignCommand {
	pub async fn run(self) -> anyhow::Result<()> {
		let source = if let Some(source) = self.source {
			source
		} else {
			let mut config = read_config().await?;
			config
				.signature_sources
				.remove(&self.key.key)
				.context("no signature source was set for this key, and no override was provided")?
		};

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
