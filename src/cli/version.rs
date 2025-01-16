use crate::{
	cli::{
		bin_dir,
		config::{read_config, write_config, CliConfig},
		files::make_executable,
		home_dir,
		reporters::run_with_reporter,
	},
	util::no_build_metadata,
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
	reporters::DownloadsReporter,
	version_matches,
};
use semver::{Version, VersionReq};
use std::{
	collections::BTreeSet,
	env::current_exe,
	path::{Path, PathBuf},
	sync::Arc,
};
use tracing::instrument;

pub fn current_version() -> Version {
	Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
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

const ENGINES_DIR: &str = "engines";

#[instrument(level = "trace")]
pub async fn get_installed_versions(engine: EngineKind) -> anyhow::Result<BTreeSet<Version>> {
	let source = engine.source();
	let path = home_dir()?.join(ENGINES_DIR).join(source.directory());
	let mut installed_versions = BTreeSet::new();

	let mut read_dir = match fs::read_dir(&path).await {
		Ok(read_dir) => read_dir,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(installed_versions),
		Err(e) => return Err(e).context("failed to read engines directory"),
	};

	while let Some(entry) = read_dir.next_entry().await? {
		let path = entry.path();

		let Some(version) = path.file_name().and_then(|s| s.to_str()) else {
			continue;
		};

		if let Ok(version) = Version::parse(version) {
			installed_versions.insert(version);
		}
	}

	Ok(installed_versions)
}

#[instrument(skip(reqwest), level = "trace")]
pub async fn get_or_download_engine(
	reqwest: &reqwest::Client,
	engine: EngineKind,
	req: VersionReq,
) -> anyhow::Result<PathBuf> {
	let source = engine.source();
	let path = home_dir()?.join(ENGINES_DIR).join(source.directory());

	let installed_versions = get_installed_versions(engine).await?;

	let max_matching = installed_versions
		.iter()
		.filter(|v| version_matches(v, &req))
		.next_back();
	if let Some(version) = max_matching {
		return Ok(path
			.join(version.to_string())
			.join(source.expected_file_name())
			.with_extension(std::env::consts::EXE_EXTENSION));
	}

	run_with_reporter(|_, root_progress, reporter| async {
		let root_progress = root_progress;
		let reporter = reporter;

		root_progress.set_message("resolve version");
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

		root_progress.set_message("download");

		let reporter = reporter.report_download(format!("{engine} v{version}"));

		let archive = source
			.download(
				&engine_ref,
				&DownloadOptions {
					reqwest: reqwest.clone(),
					reporter: Arc::new(reporter),
					version: version.clone(),
				},
			)
			.await
			.context("failed to download engine")?;

		let path = path.join(version.to_string());
		fs::create_dir_all(&path)
			.await
			.context("failed to create engine container folder")?;
		let path = path
			.join(source.expected_file_name())
			.with_extension(std::env::consts::EXE_EXTENSION);

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

		Ok::<_, anyhow::Error>(())
	})
	.await?;

	make_executable(&path)
		.await
		.context("failed to make downloaded version executable")?;

	if engine != EngineKind::Pesde {
		make_linker_if_needed(engine).await?;
	}

	Ok(path)
}

#[instrument(level = "trace")]
pub async fn replace_pesde_bin_exe(with: &Path) -> anyhow::Result<()> {
	let bin_exe_path = bin_dir()
		.await?
		.join(EngineKind::Pesde.to_string())
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

#[instrument(level = "trace")]
pub async fn make_linker_if_needed(engine: EngineKind) -> anyhow::Result<()> {
	let bin_dir = bin_dir().await?;
	let linker = bin_dir
		.join(engine.to_string())
		.with_extension(std::env::consts::EXE_EXTENSION);
	let exists = linker.exists();

	if !exists {
		let exe = current_exe().context("failed to get current exe path")?;

		#[cfg(windows)]
		let result = fs::symlink_file(exe, linker);
		#[cfg(not(windows))]
		let result = fs::symlink(exe, linker);

		result.await.context("failed to create symlink")?;
	}

	Ok(())
}
