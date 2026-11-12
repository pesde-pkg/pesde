use anyhow::Context as _;
use clap::Args;
use clap::ValueEnum;
use pesde::signature::KeyKind;

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
		let format =
			self.format
				.unwrap_or(match (self.key.was_external_format, self.key.key.kind()) {
					(true, _) => TokenFormat::Pesde,
					(false, KeyKind::Ed25519) => TokenFormat::Ssh,
					(false, _) => {
						anyhow::bail!("don't know how to format this key type, specify --format")
					}
				});

		match format {
			TokenFormat::Pesde => {
				println!("{}", self.key.key);
			}
			TokenFormat::Ssh => {
				use ssh_key::PublicKey as SshKey;
				use ssh_key::public::Ed25519PublicKey;
				use ssh_key::public::KeyData;

				let data = match self.key.key.kind() {
					KeyKind::Ed25519 => KeyData::Ed25519(Ed25519PublicKey(
						self.key
							.key
							.data()
							.try_into()
							.context("key data length should match key kind")?,
					)),
					_ => {
						anyhow::bail!("don't know how to format this key type as SSH")
					}
				};
				let key = SshKey::new(data, "");
				println!("{}", key.to_string());
			}
		}

		Ok(())
	}
}
