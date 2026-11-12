use std::str::FromStr;

use anyhow::Context as _;
use pesde::signature::PublicKey;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserInputPublicKey {
	pub key: PublicKey,
	pub was_external_format: bool,
}

impl FromStr for UserInputPublicKey {
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if s.starts_with("ssh-") {
			use ssh_key::PublicKey as SshKey;
			use ssh_key::public::KeyData;
			let key = SshKey::from_str(s)?;

			let (kind, data) = match key.key_data() {
				KeyData::Ed25519(data) => (pesde::signature::KeyKind::Ed25519, data.0.as_slice()),
				_ => return Err(anyhow::anyhow!("unsupported key algorithm")),
			};

			Ok(Self {
				key: PublicKey::new(kind, data).context("produced invalid key")?,
				was_external_format: true,
			})
		} else if let Ok(key) = PublicKey::from_str(s) {
			Ok(Self {
				key,
				was_external_format: false,
			})
		} else {
			Err(anyhow::anyhow!("invalid public key format"))
		}
	}
}
