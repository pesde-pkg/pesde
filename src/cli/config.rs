use crate::cli::{auth::Tokens, home_dir};
use anyhow::Context;
use fs_err::tokio as fs;
use serde::{Deserialize, Serialize};
use tracing::instrument;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CliConfig {
    #[serde(
        serialize_with = "crate::util::serialize_gix_url",
        deserialize_with = "crate::util::deserialize_gix_url"
    )]
    pub default_index: gix::Url,

    pub tokens: Tokens,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_updates: Option<(chrono::DateTime<chrono::Utc>, semver::Version)>,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            default_index: "https://github.com/pesde-pkg/index".try_into().unwrap(),

            tokens: Tokens(Default::default()),

            last_checked_updates: None,
        }
    }
}

#[instrument(level = "trace")]
pub async fn read_config() -> anyhow::Result<CliConfig> {
    let config_string = match fs::read_to_string(home_dir()?.join("config.toml")).await {
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
    fs::write(home_dir()?.join("config.toml"), config_string)
        .await
        .context("failed to write config file")?;

    Ok(())
}
