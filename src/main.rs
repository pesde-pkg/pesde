#[cfg(feature = "version-management")]
use crate::cli::version::{check_for_updates, current_version, get_or_download_engine};
use crate::cli::{auth::get_tokens, display_err, home_dir, HOME_DIR};
use anyhow::Context;
use clap::{builder::styling::AnsiColor, Parser};
use fs_err::tokio as fs;
use indicatif::MultiProgress;
use pesde::{engine::EngineKind, find_roots, AuthConfig, Project};
use semver::VersionReq;
use std::{
	io,
	path::{Path, PathBuf},
	str::FromStr,
	sync::Mutex,
};
use tempfile::NamedTempFile;
use tracing::instrument;
use tracing_subscriber::{
	filter::LevelFilter, fmt::MakeWriter, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

mod cli;
pub mod util;

const STYLES: clap::builder::Styles = clap::builder::Styles::styled()
	.header(AnsiColor::Yellow.on_default().underline())
	.usage(AnsiColor::Yellow.on_default().underline())
	.literal(AnsiColor::Green.on_default().bold())
	.placeholder(AnsiColor::Cyan.on_default());

#[derive(Parser, Debug)]
#[clap(
	version,
	about = "A package manager for the Luau programming language",
	long_about = "A package manager for the Luau programming language, supporting multiple runtimes including Roblox and Lune"
)]
#[command(disable_version_flag = true, styles = STYLES)]
struct Cli {
	/// Print version
	#[arg(short = 'v', short_alias = 'V', long, action = clap::builder::ArgAction::Version)]
	version: (),

	#[command(subcommand)]
	subcommand: cli::commands::Subcommand,
}

#[instrument(level = "trace")]
async fn get_linkable_dir(path: &Path) -> PathBuf {
	let mut curr_path = PathBuf::new();
	let file_to_try = NamedTempFile::new_in(path).expect("failed to create temporary file");

	let temp_path = tempfile::Builder::new()
		.make(|_| Ok(()))
		.expect("failed to create temporary file")
		.into_temp_path();
	let temp_file_name = temp_path.file_name().expect("failed to get file name");

	// C: and \ are different components on Windows
	#[cfg(windows)]
	let components = path.components().map(|c| {
		let mut path = c.as_os_str().to_os_string();
		if let std::path::Component::Prefix(_) = c {
			path.push(std::path::MAIN_SEPARATOR_STR);
		}

		path
	});
	#[cfg(not(windows))]
	let components = path.components().map(|c| c.as_os_str().to_os_string());

	for component in components {
		curr_path.push(component);

		let try_path = curr_path.join(temp_file_name);

		if fs::hard_link(file_to_try.path(), &try_path).await.is_ok() {
			if let Err(err) = fs::remove_file(&try_path).await {
				tracing::warn!(
					"failed to remove temporary file at {}: {err}",
					try_path.display()
				);
			}

			return curr_path;
		}
	}

	panic!(
		"couldn't find a linkable directory for any point in {}",
		curr_path.display()
	);
}

pub static PROGRESS_BARS: Mutex<Option<MultiProgress>> = Mutex::new(None);

#[derive(Clone, Copy)]
pub struct IndicatifWriter;

impl IndicatifWriter {
	fn suspend<F: FnOnce() -> R, R>(f: F) -> R {
		match *PROGRESS_BARS.lock().unwrap() {
			Some(ref progress_bars) => progress_bars.suspend(f),
			None => f(),
		}
	}
}

impl io::Write for IndicatifWriter {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		Self::suspend(|| io::stderr().write(buf))
	}

	fn flush(&mut self) -> io::Result<()> {
		Self::suspend(|| io::stderr().flush())
	}

	fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
		Self::suspend(|| io::stderr().write_vectored(bufs))
	}

	fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
		Self::suspend(|| io::stderr().write_all(buf))
	}

	fn write_fmt(&mut self, fmt: std::fmt::Arguments<'_>) -> io::Result<()> {
		Self::suspend(|| io::stderr().write_fmt(fmt))
	}
}

impl<'a> MakeWriter<'a> for IndicatifWriter {
	type Writer = IndicatifWriter;

	fn make_writer(&'a self) -> Self::Writer {
		*self
	}
}

async fn run() -> anyhow::Result<()> {
	let cwd = std::env::current_dir().expect("failed to get current working directory");
	let current_exe = std::env::current_exe().expect("failed to get current executable path");
	let exe_name = current_exe.file_stem().unwrap();

	#[cfg(windows)]
	'scripts: {
		// we're called the same as the binary, so we're not a (legal) script
		if exe_name == env!("CARGO_PKG_NAME") {
			break 'scripts;
		}

		if let Some(bin_folder) = current_exe.parent() {
			// we're not in {path}/bin/{exe}
			if bin_folder.file_name().is_some_and(|parent| parent != "bin") {
				break 'scripts;
			}

			// we're not in {path}/.pesde/bin/{exe}
			if bin_folder
				.parent()
				.and_then(|home_folder| home_folder.file_name())
				.is_some_and(|home_folder| home_folder != HOME_DIR)
			{
				break 'scripts;
			}
		}

		// the bin script will search for the project root itself, so we do that to ensure
		// consistency across platforms, since the script is executed using a shebang
		// on unix systems
		let status = std::process::Command::new("lune")
			.arg("run")
			.arg(
				current_exe
					.parent()
					.unwrap_or(&current_exe)
					.join(".impl")
					.join(current_exe.file_name().unwrap())
					.with_extension("luau"),
			)
			.arg("--")
			.args(std::env::args_os().skip(1))
			.current_dir(cwd)
			.status()
			.expect("failed to run lune");

		std::process::exit(status.code().unwrap());
	}

	let tracing_env_filter = EnvFilter::builder()
		.with_default_directive(LevelFilter::INFO.into())
		.from_env_lossy()
		.add_directive("reqwest=info".parse().unwrap())
		.add_directive("rustls=info".parse().unwrap())
		.add_directive("tokio_util=info".parse().unwrap())
		.add_directive("goblin=info".parse().unwrap())
		.add_directive("tower=info".parse().unwrap())
		.add_directive("hyper=info".parse().unwrap())
		.add_directive("h2=info".parse().unwrap());

	let fmt_layer = tracing_subscriber::fmt::layer().with_writer(IndicatifWriter);

	#[cfg(debug_assertions)]
	let fmt_layer = fmt_layer.with_timer(tracing_subscriber::fmt::time::uptime());

	#[cfg(not(debug_assertions))]
	let fmt_layer = fmt_layer
		.pretty()
		.with_timer(())
		.with_line_number(false)
		.with_file(false)
		.with_target(false);

	tracing_subscriber::registry()
		.with(tracing_env_filter)
		.with(fmt_layer)
		.init();

	let (project_root_dir, project_workspace_dir) = find_roots(cwd.clone())
		.await
		.context("failed to find project root")?;

	tracing::trace!(
		"project root: {}\nworkspace root: {}",
		project_root_dir.display(),
		project_workspace_dir
			.as_ref()
			.map_or("none".to_string(), |p| p.display().to_string())
	);

	let home_dir = home_dir()?;
	let data_dir = home_dir.join("data");
	fs::create_dir_all(&data_dir)
		.await
		.expect("failed to create data directory");

	let cas_dir = get_linkable_dir(&project_root_dir).await.join(HOME_DIR);

	let cas_dir = if cas_dir == home_dir {
		&data_dir
	} else {
		&cas_dir
	}
	.join("cas");

	tracing::debug!("using cas dir in {}", cas_dir.display());

	let project = Project::new(
		project_root_dir,
		project_workspace_dir,
		data_dir,
		cas_dir,
		AuthConfig::new().with_tokens(get_tokens().await?.0),
	);

	let reqwest = {
		let mut headers = reqwest::header::HeaderMap::new();

		headers.insert(
			reqwest::header::ACCEPT,
			"application/json"
				.parse()
				.context("failed to create accept header")?,
		);

		reqwest::Client::builder()
			.user_agent(concat!(
				env!("CARGO_PKG_NAME"),
				"/",
				env!("CARGO_PKG_VERSION")
			))
			.default_headers(headers)
			.build()?
	};

	#[cfg(feature = "version-management")]
	'engines: {
		let Some(engine) = exe_name
			.to_str()
			.and_then(|str| EngineKind::from_str(str).ok())
		else {
			break 'engines;
		};

		let req = project
			.deser_manifest()
			.await
			.ok()
			.and_then(|mut manifest| manifest.engines.remove(&engine));

		if engine == EngineKind::Pesde {
			match &req {
				// we're already running a compatible version
				Some(req) if req.matches(&current_version()) => break 'engines,
				// the user has not requested a specific version, so we'll just use the current one
				None => break 'engines,
				_ => (),
			}
		}

		let exe_path =
			get_or_download_engine(&reqwest, engine, req.unwrap_or(VersionReq::STAR)).await?;
		if exe_path == current_exe {
			break 'engines;
		}

		let status = std::process::Command::new(exe_path)
			.args(std::env::args_os().skip(1))
			.status()
			.expect("failed to run new version");

		std::process::exit(status.code().unwrap());
	}

	#[cfg(feature = "version-management")]
	display_err(
		check_for_updates(&reqwest).await,
		" while checking for updates",
	);

	let cli = Cli::parse();

	cli.subcommand.run(project, reqwest).await
}

#[tokio::main]
async fn main() {
	let result = run().await;
	let is_err = result.is_err();
	display_err(result, "");
	if is_err {
		std::process::exit(1);
	}
}
