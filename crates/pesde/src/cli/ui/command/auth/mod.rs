use clap::Subcommand;

mod format_key;
mod script;
mod sign;
mod signature_source;
mod verify;

/// Auth-related commands.
#[derive(Debug, Subcommand)]
pub enum AuthCommand {
	FormatKey(format_key::FormatKeyCommand),
	Script(script::ScriptCommand),
	SignatureSource(signature_source::SignatureSourceCommand),
	Sign(sign::SignCommand),
	Verify(verify::VerifyCommand),
}

impl AuthCommand {
	pub async fn run(self) -> anyhow::Result<()> {
		match self {
			Self::FormatKey(cmd) => cmd.run(),
			Self::Script(cmd) => cmd.run().await,
			Self::SignatureSource(cmd) => cmd.run().await,
			Self::Sign(cmd) => cmd.run().await,
			Self::Verify(cmd) => cmd.run().await,
		}
	}
}
