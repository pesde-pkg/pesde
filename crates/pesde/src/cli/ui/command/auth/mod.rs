use clap::Subcommand;

mod format_key;
mod script;

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
	FormatKey(format_key::FormatKeyCommand),
	Script(script::ScriptCommand),
}

impl AuthCommand {
	pub async fn run(self) -> anyhow::Result<()> {
		match self {
			Self::FormatKey(cmd) => cmd.run(),
			Self::Script(cmd) => cmd.run().await,
		}
	}
}
