use std::sync::LazyLock;

use anyhow::Context as _;
use bstr::BString;
use clap::Args;
use clap::ValueEnum;
use pesde::registry::Consent;
use pesde::registry::ScopeId;
use pesde::registry::Signable;
use pesde::registry::Signed;
use pesde::registry::Tagged;
use pesde::registry::UnvalidatedConsent;
use pesde::registry::UnvalidatedSigned;
use pesde::signature::PublicKey;
use pesde::signature::PublicKeyAlgorithm;
use pesde::signature::Signature;
use pesde::signature::SignatureKind;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;

use crate::cli::ui::CommandContext;
use crate::cli::ui::public_key::PublicKeyExt as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureSource {
	#[value(
		aliases = ["ssh_agent", "sshagent"],
		help = "the public key will be required to be representable in the SSH format and the signature will be requested from the SSH agent"
	)]
	SshAgent,
}

impl SignatureSource {
	pub async fn sign(self, key: &PublicKey, message: &[u8]) -> anyhow::Result<Signature> {
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

				static SSH_AGENT_PATH: LazyLock<Option<std::path::PathBuf>> =
					LazyLock::new(|| std::env::var_os("SSH_AUTH_SOCK").map(Into::into));

				let ssh_agent_path = SSH_AGENT_PATH.as_deref().map_or_else(
					|| {
						#[cfg(windows)]
						{
							anyhow::Ok(std::path::Path::new(r"\\.\pipe\openssh-ssh-agent"))
						}
						#[cfg(not(windows))]
						{
							anyhow::bail!(
								"SSH_AUTH_SOCK is not set, and no default path is available for this platform"
							)
						}
					},
					Ok,
				)?;

				#[cfg(windows)]
				let mut client = tokio::net::windows::named_pipe::ClientOptions::new()
					.open(ssh_agent_path)
					.context("failed to open SSH agent named pipe")?;
				#[cfg(unix)]
				let mut client = tokio::net::UnixStream::connect(ssh_agent_path)
					.await
					.context("failed to connect to SSH agent socket")?;

				client
					.write_all(&agent_message)
					.await
					.context("failed to write to SSH agent")?;
				client
					.flush()
					.await
					.context("failed to flush SSH agent write buffer")?;

				let len = client
					.read_u32()
					.await
					.context("failed to read response length from SSH agent")?;

				let payload_len = len
					.checked_sub(1)
					.context("SSH agent response length is too small")?;
				let payload_len = usize::try_from(payload_len)
					.context("SSH agent response length is too large")?;
				let status = client
					.read_u8()
					.await
					.context("failed to read status from SSH agent")?;

				match status {
					// SSH_AGENT_SIGN_RESPONSE
					14 => {
						let mut ssh_string = vec![0; payload_len];
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
						let mut payload = vec![0; payload_len];
						if client.read_exact(&mut payload).await.is_ok() {
							tracing::debug!(
								"SSH agent returned unexpected status code {s} with payload: {:?}",
								BString::from(payload)
							);
						}
						anyhow::bail!("SSH agent returned unexpected status code: {s}");
					}
				}
			}
		}
		.and_then(|sig| {
			sig.verify(key, message)
				.then_some(sig)
				.context("signature failed to verify")
		})
	}
}

#[derive(Debug, Args)]
pub struct SignatureSourceArgs {
	/// An override for the signature source to use. If not provided, the signature source associated with the key is used
	#[arg(long, aliases = ["sig-source", "signature-source", "sigsrc"])]
	signature_source: Option<SignatureSource>,
}

impl SignatureSourceArgs {
	pub async fn get_source_from_config(
		&self,
		key: &PublicKey,
		signature_source: impl AsyncFnOnce(&PublicKey) -> anyhow::Result<Option<SignatureSource>>,
	) -> anyhow::Result<SignatureSource> {
		if let Some(source) = self.signature_source {
			Ok(source)
		} else {
			signature_source(key)
				.await?
				.context("no signature source was set for this key, and no override was provided")
		}
	}

	pub async fn get_source(
		&self,
		ctx: &CommandContext,
		key: &PublicKey,
	) -> anyhow::Result<SignatureSource> {
		self.get_source_from_config(key, async |key| {
			Ok(ctx.cli_config().await?.signature_sources.get(key).copied())
		})
		.await
	}

	pub async fn sign(
		&self,
		ctx: &CommandContext,
		key: &PublicKey,
		message: &[u8],
	) -> anyhow::Result<Signature> {
		let source = self.get_source(ctx, key).await?;
		source.sign(key, message).await
	}

	pub async fn signed<T: Signable>(
		&self,
		ctx: &CommandContext,
		body: T,
	) -> anyhow::Result<Signed<T>> {
		let sig = self
			.sign(ctx, body.signer(), &T::signing_bytes(&body))
			.await?;

		let signed = UnvalidatedSigned { sig, body };
		let signed = Signed::<T>::new(signed).context("failed to validate signature")?;

		Ok(signed)
	}

	pub async fn consent<T: Tagged>(
		&self,
		ctx: &CommandContext,
		scope_id: &ScopeId,
		consenter: PublicKey,
	) -> anyhow::Result<Consent<T>> {
		let consent = UnvalidatedConsent {
			nonce: rand::random(),
			consenter,
		};

		let sig = self
			.sign(
				ctx,
				&consent.consenter,
				&Consent::<T>::signing_bytes(scope_id, &consent),
			)
			.await?;

		let consent = Consent::<T>::new(scope_id, UnvalidatedSigned { sig, body: consent })
			.context("failed to validate signature")?;

		Ok(consent)
	}
}
