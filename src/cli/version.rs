use anyhow::Context;
use colored::Colorize;
use reqwest::header::ACCEPT;
use semver::Version;
use serde::Deserialize;
use std::{io::Read, path::PathBuf};

use crate::cli::{
    bin_dir,
    config::{read_config, write_config, CliConfig},
    files::make_executable,
    home_dir,
};

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    url: url::Url,
}

fn get_repo() -> (String, String) {
    let mut parts = env!("CARGO_PKG_REPOSITORY").split('/').skip(3);
    (
        parts.next().unwrap().to_string(),
        parts.next().unwrap().to_string(),
    )
}

const CHECK_INTERVAL: chrono::Duration = chrono::Duration::hours(6);

pub fn check_for_updates(reqwest: &reqwest::blocking::Client) -> anyhow::Result<()> {
    let (owner, repo) = get_repo();

    let config = read_config()?;

    let version = if let Some((_, version)) = config
        .last_checked_updates
        .filter(|(time, _)| chrono::Utc::now() - *time < CHECK_INTERVAL)
    {
        version
    } else {
        let releases = reqwest
            .get(format!(
                "https://api.github.com/repos/{owner}/{repo}/releases",
            ))
            .send()
            .context("failed to send request to GitHub API")?
            .error_for_status()
            .context("failed to get GitHub API response")?
            .json::<Vec<Release>>()
            .context("failed to parse GitHub API response")?;

        let version = releases
            .into_iter()
            .map(|release| Version::parse(release.tag_name.trim_start_matches('v')).unwrap())
            .max()
            .context("failed to find latest version")?;

        write_config(&CliConfig {
            last_checked_updates: Some((chrono::Utc::now(), version.clone())),
            ..config
        })?;

        version
    };

    if version > current_version() {
        let name = env!("CARGO_PKG_NAME");

        let unformatted_message = format!("a new version of {name} is available: {version}");

        let message = format!(
            "a new version of {} is available: {}",
            name.cyan(),
            version.to_string().yellow().bold()
        );

        let stars = "-"
            .repeat(unformatted_message.len() + 4)
            .bright_magenta()
            .bold();
        let column = "|".bright_magenta().bold();

        println!("\n{stars}\n{column} {message} {column}\n{stars}\n");
    }

    Ok(())
}

pub fn download_github_release(
    reqwest: &reqwest::blocking::Client,
    version: &Version,
) -> anyhow::Result<Vec<u8>> {
    let (owner, repo) = get_repo();

    let release = reqwest
        .get(format!(
            "https://api.github.com/repos/{owner}/{repo}/releases/tags/v{version}",
        ))
        .send()
        .context("failed to send request to GitHub API")?
        .error_for_status()
        .context("failed to get GitHub API response")?
        .json::<Release>()
        .context("failed to parse GitHub API response")?;

    let asset = release
        .assets
        .into_iter()
        .find(|asset| {
            asset.name.ends_with(&format!(
                "-{}-{}.tar.gz",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
        })
        .context("failed to find asset for current platform")?;

    let bytes = reqwest
        .get(asset.url)
        .header(ACCEPT, "application/octet-stream")
        .send()
        .context("failed to send request to download asset")?
        .error_for_status()
        .context("failed to download asset")?
        .bytes()
        .context("failed to download asset")?;

    let mut decoder = flate2::read::GzDecoder::new(bytes.as_ref());
    let mut archive = tar::Archive::new(&mut decoder);

    let entry = archive
        .entries()
        .context("failed to read archive entries")?
        .next()
        .context("archive has no entry")?
        .context("failed to get first archive entry")?;

    entry
        .bytes()
        .collect::<Result<Vec<u8>, std::io::Error>>()
        .context("failed to read archive entry bytes")
}

pub fn get_or_download_version(
    reqwest: &reqwest::blocking::Client,
    version: &Version,
) -> anyhow::Result<Option<PathBuf>> {
    let path = home_dir()?.join("versions");
    fs_err::create_dir_all(&path).context("failed to create versions directory")?;

    let path = path.join(format!("{version}{}", std::env::consts::EXE_SUFFIX));

    let is_requested_version = *version == current_version();

    if path.exists() {
        return Ok(if is_requested_version {
            None
        } else {
            Some(path)
        });
    }

    if is_requested_version {
        fs_err::copy(std::env::current_exe()?, &path)
            .context("failed to copy current executable to version directory")?;
    } else {
        let bytes = download_github_release(reqwest, version)?;
        fs_err::write(&path, bytes).context("failed to write downloaded version file")?;
    }

    make_executable(&path).context("failed to make downloaded version executable")?;

    Ok(if is_requested_version {
        None
    } else {
        Some(path)
    })
}

pub fn max_installed_version() -> anyhow::Result<Version> {
    let versions_dir = home_dir()?.join("versions");
    fs_err::create_dir_all(&versions_dir).context("failed to create versions directory")?;

    let max_version = fs_err::read_dir(versions_dir)
        .context("failed to read versions directory")?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|entry| {
            #[cfg(not(windows))]
            let name = entry
                .path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            #[cfg(windows)]
            let name = entry
                .path()
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string();

            Version::parse(&name).unwrap()
        })
        .max()
        .filter(|v| v >= &current_version())
        .unwrap_or_else(current_version);

    Ok(max_version)
}

pub fn update_bin_exe() -> anyhow::Result<()> {
    let copy_to = bin_dir()?.join(format!(
        "{}{}",
        env!("CARGO_BIN_NAME"),
        std::env::consts::EXE_SUFFIX
    ));

    fs_err::copy(std::env::current_exe()?, &copy_to)
        .context("failed to copy executable to bin folder")?;

    make_executable(&copy_to)
}
