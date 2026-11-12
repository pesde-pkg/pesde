use crate::cli::config::read_config;
use crate::cli::config::write_config;
use anyhow::Context as _;
use clap::Args;
use pesde::Url;

/// Sets a script that returns the value to set for `Authorization` headers to the given URL
#[derive(Debug, Args)]
pub struct ScriptCommand {
	/// The URL to set the command for
	url: Url,

	/// The script to execute.
	/// If non-empty, it is set. If empty, the script is removed. If unset, the current script is printed.
	///
	/// Warning: this script is stored in plaintext. All secrets should be handled by the script itself, e.g. through `cat`
	script: Option<Box<str>>,
}

impl ScriptCommand {
	pub async fn run(self) -> anyhow::Result<()> {
		let mut config = read_config().await?;

		if let Some(script) = self.script {
			if script.is_empty() {
				if config.auth_scripts.remove(&self.url).is_none() {
					println!("no auth script was set");
				}
			} else {
				config.auth_scripts.insert(self.url, script);
			}

			write_config(&config).await?;
		} else {
			let script = config
				.auth_scripts
				.get(&self.url)
				.context("no auth script was set")?;
			println!("{script}");
		}

		Ok(())
	}
}
