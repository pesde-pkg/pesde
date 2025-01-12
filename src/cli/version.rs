use crate::cli::{
	bin_dir,
	config::{read_config, write_config, CliConfig},
	files::make_executable,
	home_dir,
};
use anyhow::Context;
use colored::Colorize;
use fs_err::tokio as fs;
use pesde::{
	engine::{
		source::{
			traits::{DownloadOptions, EngineSource, ResolveOptions},
			EngineSources,
		},
		EngineKind,
	},
	version_matches,
};
use semver::{Version, VersionReq};
use std::{
	collections::BTreeSet,
	path::{Path, PathBuf},
	sync::Arc,
};
use tracing::instrument;

pub fn current_version() -> Version {
	Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
}

pub fn no_build_metadata(version: &Version) -> Version {
	let mut version = version.clone();
	version.build = semver::BuildMetadata::EMPTY;
	version
}

const CHECK_INTERVAL: chrono::Duration = chrono::Duration::hours(6);

pub async fn find_latest_version(reqwest: &reqwest::Client) -> anyhow::Result<Version> {
	let version = EngineSources::pesde()
		.resolve(
			&VersionReq::STAR,
			&ResolveOptions {
				reqwest: reqwest.clone(),
			},
		)
		.await
		.context("failed to resolve version")?
		.pop_last()
		.context("no versions found")?
		.0;

	Ok(version)
}

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
		let version = find_latest_version(reqwest).await?;

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

#[instrument(skip(reqwest), level = "trace")]
pub async fn get_or_download_engine(
	reqwest: &reqwest::Client,
	engine: EngineKind,
	req: VersionReq,
) -> anyhow::Result<PathBuf> {
	let source = engine.source();

	let path = home_dir()?.join("engines").join(source.directory());
	fs::create_dir_all(&path)
		.await
		.context("failed to create engines directory")?;

	let mut read_dir = fs::read_dir(&path)
		.await
		.context("failed to read engines directory")?;

	let mut matching_versions = BTreeSet::new();

	while let Some(entry) = read_dir.next_entry().await? {
		let path = entry.path();

		#[cfg(windows)]
		let version = path.file_stem();
		#[cfg(not(windows))]
		let version = path.file_name();

		let Some(version) = version.and_then(|s| s.to_str()) else {
			continue;
		};

		if let Ok(version) = Version::parse(version) {
			if version_matches(&version, &req) {
				matching_versions.insert(version);
			}
		}
	}

	if let Some(version) = matching_versions.pop_last() {
		return Ok(path
			.join(version.to_string())
			.join(source.expected_file_name())
			.with_extension(std::env::consts::EXE_EXTENSION));
	}

	let mut versions = source
		.resolve(
			&req,
			&ResolveOptions {
				reqwest: reqwest.clone(),
			},
		)
		.await
		.context("failed to resolve versions")?;
	let (version, engine_ref) = versions.pop_last().context("no matching versions found")?;

	let path = path
		.join(version.to_string())
		.join(source.expected_file_name())
		.with_extension(std::env::consts::EXE_EXTENSION);

	let archive = source
		.download(
			&engine_ref,
			&DownloadOptions {
				reqwest: reqwest.clone(),
				reporter: Arc::new(()),
				version,
			},
		)
		.await
		.context("failed to download engine")?;

	let mut file = fs::File::create(&path)
		.await
		.context("failed to create new file")?;
	tokio::io::copy(
		&mut archive
			.find_executable(source.expected_file_name())
			.await
			.context("failed to find executable")?,
		&mut file,
	)
	.await
	.context("failed to write to file")?;

	make_executable(&path)
		.await
		.context("failed to make downloaded version executable")?;

	Ok(path)
}

#[instrument(level = "trace")]
pub async fn replace_bin_exe(engine: EngineKind, with: &Path) -> anyhow::Result<()> {
	let bin_exe_path = bin_dir()
		.await?
		.join(engine.to_string())
		.with_extension(std::env::consts::EXE_EXTENSION);

	let exists = bin_exe_path.exists();

	if cfg!(target_os = "linux") && exists {
		fs::remove_file(&bin_exe_path)
			.await
			.context("failed to remove existing executable")?;
	} else if exists {
		let tempfile = tempfile::Builder::new()
			.make(|_| Ok(()))
			.context("failed to create temporary file")?;
		let temp_path = tempfile.into_temp_path().to_path_buf();
		#[cfg(windows)]
		let temp_path = temp_path.with_extension("exe");

		match fs::rename(&bin_exe_path, &temp_path).await {
			Ok(_) => {}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(e).context("failed to rename existing executable"),
		}
	}

	fs::copy(with, &bin_exe_path)
		.await
		.context("failed to copy executable to bin folder")?;

	make_executable(&bin_exe_path).await
}
