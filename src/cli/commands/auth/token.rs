use crate::cli::auth::get_tokens;
use crate::cli::commands::auth::get_index;
use clap::Args;
use pesde::Subproject;

#[derive(Debug, Args)]
pub struct TokenCommand {
	/// The index to use. Defaults to `default`, or the configured default index if current directory doesn't have a manifest
	#[arg(short, long)]
	index: Option<String>,
}

impl TokenCommand {
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		let index_url = get_index(&subproject, self.index.as_deref()).await?;

		let tokens = get_tokens().await?;
		let Some(token) = tokens.get(&index_url) else {
			eprintln!("not logged in into {index_url}");
			return Ok(());
		};

		eprintln!("token for {index_url}:");
		println!("{token}");

		Ok(())
	}
}
