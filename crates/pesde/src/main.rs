use std::sync::Mutex;

use indicatif::MultiProgress;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use clap::Parser;
use clap::builder::styling::AnsiColor;

mod cli;

const STYLES: clap::builder::Styles = clap::builder::Styles::styled()
	.header(AnsiColor::Yellow.on_default().underline())
	.usage(AnsiColor::Yellow.on_default().underline())
	.literal(AnsiColor::Green.on_default().bold())
	.placeholder(AnsiColor::Cyan.on_default());

#[derive(Parser, Debug)]
#[command(about = "A package manager for the Luau programming language", styles = STYLES, version, disable_version_flag = true)]
struct Cli {
	/// Print version
	#[arg(short = 'v', short_alias = 'V', long, action = clap::builder::ArgAction::Version)]
	version: (),

	/// The subcommand to run
	#[command(subcommand)]
	command: cli::ui::Command,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

	let cli = Cli::parse();
	cli.command.run(cli::ui::CommandContext::new()).await
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

impl std::io::Write for IndicatifWriter {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		Self::suspend(|| std::io::stderr().write(buf))
	}

	fn flush(&mut self) -> std::io::Result<()> {
		Self::suspend(|| std::io::stderr().flush())
	}

	fn write_vectored(&mut self, bufs: &[std::io::IoSlice<'_>]) -> std::io::Result<usize> {
		Self::suspend(|| std::io::stderr().write_vectored(bufs))
	}

	fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
		Self::suspend(|| std::io::stderr().write_all(buf))
	}

	fn write_fmt(&mut self, args: std::fmt::Arguments<'_>) -> std::io::Result<()> {
		Self::suspend(|| std::io::stderr().write_fmt(args))
	}
}

impl<'a> MakeWriter<'a> for IndicatifWriter {
	type Writer = IndicatifWriter;

	fn make_writer(&'a self) -> Self::Writer {
		*self
	}
}
