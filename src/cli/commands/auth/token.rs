use crate::cli::auth::get_tokens;
use clap::Args;
use pesde::GixUrl;

#[derive(Debug, Args)]
pub struct TokenCommand;

impl TokenCommand {
	pub async fn run(self, index_url: GixUrl) -> anyhow::Result<()> {
		let tokens = get_tokens().await?;
		let Some(token) = tokens.get(&index_url) else {
			println!("not logged in into {index_url}");
			return Ok(());
		};

		println!("token for {index_url}: \"{token}\"");

		Ok(())
	}
}
