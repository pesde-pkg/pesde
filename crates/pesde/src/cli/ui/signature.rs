use std::sync::LazyLock;

use anyhow::Context as _;
use clap::ValueEnum;
use pesde::signature::{PublicKey, Signature};
use pesde::signature::{PublicKeyAlgorithm, SignatureKind};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;

use crate::cli::ui::public_key::PublicKeyExt as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureSource {
	#[value(
		alias = "ssh_agent",
		help = "the public key will be converted to the SSH format, if possible, and the signature will be requested from the SSH agent"
	)]
	SshAgent,
}

impl SignatureSource {
	pub async fn sign(&self, key: &PublicKey, message: &[u8]) -> anyhow::Result<Signature> {
		match self {
			SignatureSource::SshAgent => {
				use ssh_encoding::Decode as _;
				use ssh_encoding::Encode as _;

				let expected_signature_kind = match key.algorithm() {
					PublicKeyAlgorithm::Ed25519 => SignatureKind::SshEd25519,
					_ => anyhow::bail!("SSH agent signing is only supported for Ed25519 keys"),
				};

				let key = key
					.as_ssh_key()
					.context("key type cannot be converted to SSH format")?;

				let sign_data = ssh_key::SshSig::signed_data(
					Signature::SSH_NAMESPACE,
					ssh_key::HashAlg::Sha512,
					message,
				)
				.context("failed to compute SSH signature data")?;

				let mut agent_message = vec![];

				// length, u32 BE
				agent_message.extend(0u32.to_be_bytes());
				// SSH_AGENTC_SIGN_REQUEST
				13u8.encode(&mut agent_message)?;
				// key
				key.key_data().encode_prefixed(&mut agent_message)?;
				// data
				sign_data.encode(&mut agent_message)?;
				// flags
				0u32.encode(&mut agent_message)?;

				let len = (agent_message.len() - 4) as u32;
				agent_message[0..4].copy_from_slice(&len.to_be_bytes());

				static SSH_AGENT_PATH: LazyLock<std::path::PathBuf> = LazyLock::new(|| {
					std::env::var_os("SSH_AUTH_SOCK").map_or_else(
						|| {
							#[cfg(windows)]
							{
								std::path::PathBuf::from(r"\\.\pipe\openssh-ssh-agent")
							}
							#[cfg(not(windows))]
							{
								panic!(
									"SSH_AUTH_SOCK is not set, and no default path is available for this platform"
								)
							}
						},
						Into::into,
					)
				});

				#[cfg(windows)]
				let mut client = tokio::net::windows::named_pipe::ClientOptions::new()
					.open(&**SSH_AGENT_PATH)
					.context("failed to open SSH agent named pipe")?;
				#[cfg(unix)]
				let mut client = tokio::net::UnixStream::connect(&**SSH_AGENT_PATH)
					.await
					.context("failed to connect to SSH agent socket")?;

				client
					.write_all(&agent_message)
					.await
					.context("failed to write to SSH agent")?;
				client
					.flush()
					.await
					.context("failed to flush SSH agent named pipe")?;

				let len = client
					.read_u32()
					.await
					.context("failed to read response length from SSH agent")?;

				let status = client
					.read_u8()
					.await
					.context("failed to read status from SSH agent")?;

				match status {
					// SSH_AGENT_SIGN_RESPONSE
					14 => {
						let mut ssh_string = vec![0; len as usize - 1];
						client
							.read_exact(&mut ssh_string)
							.await
							.context("failed to read signature from SSH agent")?;

						let signature_blob = Vec::decode(&mut ssh_string.as_slice()).context(
							"failed to decode SSH signature blob from SSH agent response",
						)?;

						let signature = ssh_key::Signature::decode(&mut signature_blob.as_slice())
							.context("failed to decode SSH signature from SSH agent")?;

						match (expected_signature_kind, signature.algorithm()) {
							(SignatureKind::SshEd25519, ssh_key::Algorithm::Ed25519) => {}
							_ => anyhow::bail!(
								"SSH agent returned signature with unexpected algorithm: {}",
								signature.algorithm()
							),
						}

						Signature::new(expected_signature_kind, signature.as_bytes())
							.context("failed to construct signature from SSH agent response")
					}
					// SSH_AGENT_FAILURE
					5 => {
						anyhow::bail!("SSH agent failed to sign the message");
					}
					s => {
						anyhow::bail!("SSH agent returned unexpected status code: {s}");
					}
				}
			}
		}
	}
}
