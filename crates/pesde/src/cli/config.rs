use std::collections::HashMap;

use anyhow::Context as _;
use pesde::Url;
use pesde::signature::PublicKey;
use serde::Deserialize;
use serde::Serialize;
use tracing::instrument;

use crate::cli::dirs::config_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CliConfig {
	pub default_registry: Url,

	pub auth_scripts: HashMap<Url, Box<str>>,
	#[serde(skip_serializing_if = "HashMap::is_empty")]
	pub identities: HashMap<Url, PublicKey>,
}

impl Default for CliConfig {
	fn default() -> Self {
		Self {
			default_registry: "https://registry.pesde.dev".parse().unwrap(),

			auth_scripts: Default::default(),
			identities: Default::default(),
		}
	}
}

#[instrument(level = "trace")]
pub async fn read_config() -> anyhow::Result<CliConfig> {
	let config_string = match tokio::fs::read_to_string(config_path()?).await {
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
		tokio::fs::create_dir_all(parent)
			.await
			.context("failed to create config parent directories")?;
	}

	tokio::fs::write(config_path, config_string)
		.await
		.context("failed to write config file")?;

	Ok(())
}
