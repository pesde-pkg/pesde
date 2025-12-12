use crate::cli::config::{CliConfig, read_config, write_config};
use clap::Args;

#[derive(Debug, Args)]
pub struct DefaultIndexCommand {
	/// The new index URL to set as default, don't pass any value to check the current default index
	#[arg(index = 1, value_parser = crate::cli::parse_gix_url)]
	index: Option<gix::Url>,

	/// Resets the default index to the default value
	#[arg(short, long, conflicts_with = "index")]
	reset: bool,
}

impl DefaultIndexCommand {
	pub async fn run(self) -> anyhow::Result<()> {
		let mut config = read_config().await?;

		let index = if self.reset {
			Some(CliConfig::default().default_index)
		} else {
			self.index
		};

		match index {
			Some(index) => {
				config.default_index = index.clone();
				write_config(&config).await?;
				println!("default index set to: {index}");
			}
			None => {
				println!("current default index: {}", config.default_index);
			}
		}

		Ok(())
	}
}
