use crate::cli::config::{read_config, write_config};
use clap::Args;

#[derive(Debug, Args)]
pub struct GlobalBinariesCommand {
	/// Whether to enable global binaries
	#[arg(index = 1)]
	enabled: Option<bool>,
}

impl GlobalBinariesCommand {
	pub async fn run(self) -> anyhow::Result<()> {
		let mut config = read_config().await?;
		if let Some(enabled) = self.enabled {
			config.global_binaries = enabled;
			write_config(&config).await?;
		}

		if config.global_binaries {
			println!("global binary linker generation enabled");
		} else {
			println!("global binary linker generation disabled");
		}

		Ok(())
	}
}
