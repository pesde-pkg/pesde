use anyhow::Context as _;
use clap::Args;
use clap::ValueEnum;
use pesde::signature::PublicKeyAlgorithm;

use crate::cli::ui::public_key::PublicKeyExt as _;
use crate::cli::ui::public_key::UserInputPublicKey;

/// Format a public key into a different format
#[derive(Debug, Args)]
pub struct FormatKeyCommand {
	/// The format to output the key in
	#[arg(short, long, value_enum)]
	format: Option<TokenFormat>,

	/// The key to format
	key: UserInputPublicKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum TokenFormat {
	Pesde,
	Ssh,
}

impl FormatKeyCommand {
	pub fn run(self) -> anyhow::Result<()> {
		let format = self.format.unwrap_or(
			match (self.key.was_external_format, self.key.key.algorithm()) {
				(true, _) => TokenFormat::Pesde,
				(false, PublicKeyAlgorithm::Ed25519) => TokenFormat::Ssh,
				(false, _) => {
					anyhow::bail!("don't know how to format this key type, specify --format")
				}
			},
		);

		match format {
			TokenFormat::Pesde => {
				println!("{}", self.key.key);
			}
			TokenFormat::Ssh => {
				let key = self
					.key
					.key
					.as_ssh_key()
					.context("key type cannot be converted to SSH format")?;
				println!("{}", key.to_string());
			}
		}

		Ok(())
	}
}
