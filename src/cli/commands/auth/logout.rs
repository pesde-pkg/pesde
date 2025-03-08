use crate::cli::auth::set_token;
use clap::Args;

#[derive(Debug, Args)]
pub struct LogoutCommand;

impl LogoutCommand {
	pub async fn run(self, index_url: gix::Url) -> anyhow::Result<()> {
		set_token(&index_url, None).await?;

		println!("logged out of {index_url}");

		Ok(())
	}
}
