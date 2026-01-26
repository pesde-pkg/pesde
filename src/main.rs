#[cfg(feature = "version-management")]
use crate::cli::version::{check_for_updates, current_version, get_or_download_engine};
use crate::cli::{ExecReplace as _, PESDE_DIR, auth::get_tokens, display_err};
use anyhow::Context as _;
use clap::{Parser, builder::styling::AnsiColor};
use cli::data_dir;
use fs_err::tokio as fs;
use indicatif::MultiProgress;
use pesde::{AuthConfig, Project, engine::EngineKind, find_roots};
use std::{
	io,
	path::{Path, PathBuf},
	str::FromStr as _,
	sync::Mutex,
};
use tempfile::NamedTempFile;
use tracing::instrument;
use tracing_subscriber::{
	EnvFilter, filter::LevelFilter, fmt::MakeWriter, layer::SubscriberExt as _,
	util::SubscriberInitExt as _,
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
		match &*PROGRESS_BARS.lock().unwrap() {
			Some(progress_bars) => progress_bars.suspend(f),
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

	fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> io::Result<()> {
		Self::suspend(|| io::stderr().write_fmt(args))
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
	// Unix doesn't return the symlinked path, so we need to get it from the 0 argument
	#[cfg(unix)]
	let current_exe = PathBuf::from(std::env::args_os().next().expect("argument 0 not set"));
	#[cfg(not(unix))]
	let current_exe = std::env::current_exe().expect("failed to get current executable path");
	let exe_name = current_exe
		.file_stem()
		.unwrap()
		.to_str()
		.expect("exe name is not valid utf-8");
	let exe_name_engine = EngineKind::from_str(exe_name);

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

	let (project_dir, importer) = find_roots(cwd.clone())
		.await
		.context("failed to find project root")?;

	tracing::trace!(
		"project directory: {}\nimporter: {}",
		project_dir.display(),
		if importer.as_path().as_str().is_empty() {
			"(root)"
		} else {
			importer.as_path().as_str()
		}
	);

	let reqwest = reqwest::Client::builder()
		.user_agent(concat!(
			env!("CARGO_PKG_NAME"),
			"/",
			env!("CARGO_PKG_VERSION")
		))
		.build()?;

	let cas_dir = get_linkable_dir(&project_dir)
		.await
		.join(PESDE_DIR)
		.join("cas");

	tracing::debug!("using cas dir in {}", cas_dir.display());

	let subproject = Project::new(
		project_dir,
		data_dir()?,
		cas_dir,
		AuthConfig::new().with_tokens(get_tokens().await?),
	)
	.subproject(importer);

	#[cfg(feature = "version-management")]
	'engines: {
		let Ok(engine) = exe_name_engine else {
			break 'engines;
		};

		let req = match subproject
			.deser_manifest()
			.await
			.map_err(pesde::errors::ManifestReadError::into_inner)
		{
			Ok(manifest) => manifest.engines.get(&engine).cloned(),
			Err(pesde::errors::ManifestReadErrorKind::Io(e))
				if e.kind() == io::ErrorKind::NotFound =>
			{
				None
			}
			Err(e) => return Err(e.into()),
		};

		if engine == EngineKind::Pesde {
			match &req {
				// we're already running a compatible version
				Some(req) if pesde::version_matches(req, &current_version()) => break 'engines,
				// the user has not requested a specific version, so we'll just use the current one
				None => break 'engines,
				_ => (),
			}
		}

		let exe_path = get_or_download_engine(
			&reqwest,
			engine,
			req.unwrap_or(semver::VersionReq::STAR),
			().into(),
			subproject.project().auth_config(),
		)
		.await?
		.0;
		if exe_path == current_exe {
			anyhow::bail!("engine linker executed by itself")
		}

		let mut command = std::process::Command::new(exe_path);
		command.args(std::env::args_os().skip(1));
		command.exec_replace();
	};

	#[cfg(feature = "version-management")]
	display_err(
		check_for_updates(&reqwest, subproject.project().auth_config()).await,
		" while checking for updates",
	);

	let cli = Cli::parse();

	cli.subcommand.run(subproject, reqwest).await
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
