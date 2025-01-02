use crate::cli::{
	bin_dir,
	config::{read_config, write_config, CliConfig},
	files::make_executable,
	home_dir,
};
use anyhow::Context;
use colored::Colorize;
use fs_err::tokio as fs;
use futures::StreamExt;
use reqwest::header::ACCEPT;
use semver::Version;
use serde::Deserialize;
use std::{
	env::current_exe,
	path::{Path, PathBuf},
};
use tokio::io::AsyncWrite;
use tracing::instrument;

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

#[instrument(level = "trace")]
fn get_repo() -> (String, String) {
	let mut parts = env!("CARGO_PKG_REPOSITORY").split('/').skip(3);
	let (owner, repo) = (
		parts.next().unwrap().to_string(),
		parts.next().unwrap().to_string(),
	);

	tracing::trace!("repository for updates: {owner}/{repo}");

	(owner, repo)
}

#[derive(Debug)]
pub enum VersionType {
	Latest,
	Specific(Version),
}

#[instrument(skip(reqwest), level = "trace")]
pub async fn get_remote_version(
	reqwest: &reqwest::Client,
	ty: VersionType,
) -> anyhow::Result<Version> {
	let (owner, repo) = get_repo();

	let mut releases = reqwest
		.get(format!(
			"https://api.github.com/repos/{owner}/{repo}/releases",
		))
		.send()
		.await
		.context("failed to send request to GitHub API")?
		.error_for_status()
		.context("failed to get GitHub API response")?
		.json::<Vec<Release>>()
		.await
		.context("failed to parse GitHub API response")?
		.into_iter()
		.filter_map(|release| Version::parse(release.tag_name.trim_start_matches('v')).ok());

	match ty {
		VersionType::Latest => releases.max(),
		VersionType::Specific(version) => {
			releases.find(|v| no_build_metadata(v) == no_build_metadata(&version))
		}
	}
	.context("failed to find latest version")
}

pub fn no_build_metadata(version: &Version) -> Version {
	let mut version = version.clone();
	version.build = semver::BuildMetadata::EMPTY;
	version
}

const CHECK_INTERVAL: chrono::Duration = chrono::Duration::hours(6);

#[instrument(skip(reqwest), level = "trace")]
pub async fn check_for_updates(reqwest: &reqwest::Client) -> anyhow::Result<()> {
	let config = read_config().await?;

	let version = if let Some((_, version)) = config
		.last_checked_updates
		.filter(|(time, _)| chrono::Utc::now() - *time < CHECK_INTERVAL)
	{
		tracing::debug!("using cached version");
		version
	} else {
		tracing::debug!("checking for updates");
		let version = get_remote_version(reqwest, VersionType::Latest).await?;

		write_config(&CliConfig {
			last_checked_updates: Some((chrono::Utc::now(), version.clone())),
			..config
		})
		.await?;

		version
	};
	let current_version = current_version();
	let version_no_metadata = no_build_metadata(&version);

	if version_no_metadata <= current_version {
		return Ok(());
	}

	let name = env!("CARGO_BIN_NAME");
	let changelog = format!("{}/releases/tag/v{version}", env!("CARGO_PKG_REPOSITORY"));

	let unformatted_messages = [
		"".to_string(),
		format!("update available! {current_version} → {version_no_metadata}"),
		format!("changelog: {changelog}"),
		format!("run `{name} self-upgrade` to upgrade"),
		"".to_string(),
	];

	let width = unformatted_messages
		.iter()
		.map(|s| s.chars().count())
		.max()
		.unwrap()
		+ 4;

	let column = "│".bright_magenta();

	let message = [
		"".to_string(),
		format!(
			"update available! {} → {}",
			current_version.to_string().red(),
			version_no_metadata.to_string().green()
		),
		format!("changelog: {}", changelog.blue()),
		format!(
			"run `{} {}` to upgrade",
			name.blue(),
			"self-upgrade".yellow()
		),
		"".to_string(),
	]
	.into_iter()
	.enumerate()
	.map(|(i, s)| {
		let text_length = unformatted_messages[i].chars().count();
		let padding = (width as f32 - text_length as f32) / 2f32;
		let padding_l = " ".repeat(padding.floor() as usize);
		let padding_r = " ".repeat(padding.ceil() as usize);
		format!("{column}{padding_l}{s}{padding_r}{column}")
	})
	.collect::<Vec<_>>()
	.join("\n");

	let lines = "─".repeat(width).bright_magenta();

	let tl = "╭".bright_magenta();
	let tr = "╮".bright_magenta();
	let bl = "╰".bright_magenta();
	let br = "╯".bright_magenta();

	println!("\n{tl}{lines}{tr}\n{message}\n{bl}{lines}{br}\n");

	Ok(())
}

#[instrument(skip(reqwest, writer), level = "trace")]
pub async fn download_github_release<W: AsyncWrite + Unpin>(
	reqwest: &reqwest::Client,
	version: &Version,
	mut writer: W,
) -> anyhow::Result<()> {
	let (owner, repo) = get_repo();

	let release = reqwest
		.get(format!(
			"https://api.github.com/repos/{owner}/{repo}/releases/tags/v{version}",
		))
		.send()
		.await
		.context("failed to send request to GitHub API")?
		.error_for_status()
		.context("failed to get GitHub API response")?
		.json::<Release>()
		.await
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
		.await
		.context("failed to send request to download asset")?
		.error_for_status()
		.context("failed to download asset")?
		.bytes()
		.await
		.context("failed to download asset")?;

	let mut decoder = async_compression::tokio::bufread::GzipDecoder::new(bytes.as_ref());
	let mut archive = tokio_tar::Archive::new(&mut decoder);

	let mut entry = archive
		.entries()
		.context("failed to read archive entries")?
		.next()
		.await
		.context("archive has no entry")?
		.context("failed to get first archive entry")?;

	tokio::io::copy(&mut entry, &mut writer)
		.await
		.context("failed to write archive entry to file")
		.map(|_| ())
}

#[derive(Debug)]
pub enum TagInfo {
	Complete(Version),
	Incomplete(Version),
}

#[instrument(skip(reqwest), level = "trace")]
pub async fn get_or_download_version(
	reqwest: &reqwest::Client,
	tag: TagInfo,
	always_give_path: bool,
) -> anyhow::Result<Option<PathBuf>> {
	let path = home_dir()?.join("versions");
	fs::create_dir_all(&path)
		.await
		.context("failed to create versions directory")?;

	let version = match &tag {
		TagInfo::Complete(version) => version,
		// don't fetch the version since it could be cached
		TagInfo::Incomplete(version) => version,
	};

	let path = path.join(format!(
		"{}{}",
		no_build_metadata(version),
		std::env::consts::EXE_SUFFIX
	));

	let is_requested_version = !always_give_path && *version == current_version();

	if path.exists() {
		tracing::debug!("version already exists");

		return Ok(if is_requested_version {
			None
		} else {
			Some(path)
		});
	}

	if is_requested_version {
		tracing::debug!("copying current executable to version directory");
		fs::copy(current_exe()?, &path)
			.await
			.context("failed to copy current executable to version directory")?;
	} else {
		let version = match tag {
			TagInfo::Complete(version) => version,
			TagInfo::Incomplete(version) => {
				get_remote_version(reqwest, VersionType::Specific(version))
					.await
					.context("failed to get remote version")?
			}
		};

		tracing::debug!("downloading version");
		download_github_release(
			reqwest,
			&version,
			fs::File::create(&path)
				.await
				.context("failed to create version file")?,
		)
		.await?;
	}

	make_executable(&path)
		.await
		.context("failed to make downloaded version executable")?;

	Ok(if is_requested_version {
		None
	} else {
		Some(path)
	})
}

#[instrument(level = "trace")]
pub async fn update_bin_exe(downloaded_file: &Path) -> anyhow::Result<()> {
	let bin_exe_path = bin_dir().await?.join(format!(
		"{}{}",
		env!("CARGO_BIN_NAME"),
		std::env::consts::EXE_SUFFIX
	));
	let mut downloaded_file = downloaded_file.to_path_buf();

	let exists = bin_exe_path.exists();

	if cfg!(target_os = "linux") && exists {
		fs::remove_file(&bin_exe_path)
			.await
			.context("failed to remove existing executable")?;
	} else if exists {
		let tempfile = tempfile::Builder::new()
			.make(|_| Ok(()))
			.context("failed to create temporary file")?;
		let path = tempfile.into_temp_path().to_path_buf();
		#[cfg(windows)]
		let path = path.with_extension("exe");

		let current_exe = current_exe().context("failed to get current exe path")?;
		if current_exe == downloaded_file {
			downloaded_file = path.to_path_buf();
		}

		fs::rename(&bin_exe_path, &path)
			.await
			.context("failed to rename current executable")?;
	}

	fs::copy(downloaded_file, &bin_exe_path)
		.await
		.context("failed to copy executable to bin folder")?;

	make_executable(&bin_exe_path).await
}
