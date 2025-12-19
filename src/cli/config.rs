use crate::cli::auth::Tokens;
use anyhow::Context as _;
use fs_err::tokio as fs;
use pesde::GixUrl;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::config_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CliConfig {
	pub default_index: GixUrl,

	pub tokens: Tokens,

	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub last_checked_updates: Option<(jiff::Timestamp, semver::Version)>,
}

impl Default for CliConfig {
	fn default() -> Self {
		Self {
			default_index: GixUrl::new("https://github.com/pesde-pkg/index".try_into().unwrap()),

			tokens: Tokens::default(),

			last_checked_updates: None,
		}
	}
}

#[instrument(level = "trace")]
pub async fn read_config() -> anyhow::Result<CliConfig> {
	let config_string = match fs::read_to_string(config_path()?).await {
		Ok(config_string) => config_string,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
			return Ok(CliConfig::default());
		}
		Err(e) => return Err(e).context("failed to read config file"),
	};

	let config = toml::from_str(&config_string).context("failed to parse config file")?;

	Ok(config)
}

#[instrument(level = "trace")]
pub async fn write_config(config: &CliConfig) -> anyhow::Result<()> {
	let config_string = toml::to_string(config).context("failed to serialize config")?;
	let config_path = config_path()?;

	if let Some(parent) = config_path.parent() {
		fs::create_dir_all(parent)
			.await
			.context("failed to create config parent directories")?;
	}

	fs::write(config_path, config_string)
		.await
		.context("failed to write config file")?;

	Ok(())
}
