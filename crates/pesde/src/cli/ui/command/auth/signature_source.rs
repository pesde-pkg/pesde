use clap::Args;

use crate::cli::config::read_config;
use crate::cli::config::write_config;
use crate::cli::ui::public_key::UserInputPublicKey;
use crate::cli::ui::signature::SignatureSource;

/// Associates a signature source with a public key
#[derive(Debug, Args)]
pub struct SignatureSourceCommand {
	/// The key to associate the signature source with
	key: UserInputPublicKey,

	/// The signature source to associate with the key, or none to remove the association
	source: Option<SignatureSource>,
}

impl SignatureSourceCommand {
	pub async fn run(self) -> anyhow::Result<()> {
		let mut config = read_config().await?;

		let key = self.key.key;

		if let Some(source) = self.source {
			config.signature_sources.insert(key, source);
		} else {
			config.signature_sources.remove(&key);
		}

		write_config(&config).await?;

		Ok(())
	}
}
