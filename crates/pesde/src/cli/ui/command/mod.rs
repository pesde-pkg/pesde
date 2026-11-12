use std::sync::Arc;

use anyhow::Context as _;
use clap::Subcommand;
use pesde::Project;
use pesde::WorkspaceMember;

use crate::cli::source::CliSourceProvider;
use crate::cli::vfs::FsVfs;

mod add;
mod auth;
mod cas;
mod execute;
mod init;
mod install;
mod list;
mod outdated;
mod remove;
mod run;
mod update;

#[derive(Debug, Subcommand)]
pub enum Command {
	#[command(subcommand)]
	Auth(auth::AuthCommand),

	#[command(subcommand)]
	Cas(cas::CasCommand),

	Init(init::InitCommand),

	Add(add::AddCommand),

	Remove(remove::RemoveCommand),

	List(list::ListCommand),

	Install(install::InstallCommand),

	Outdated(outdated::OutdatedCommand),

	Update(update::UpdateCommand),

	Run(run::RunCommand),

	Execute(execute::ExecuteCommand),
}

impl Command {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		match self {
			Command::Auth(cmd) => cmd.run().await,
			Command::Cas(cmd) => cmd.run(ctx).await,
			Command::Init(cmd) => cmd.run(ctx).await,
			Command::Add(cmd) => cmd.run(ctx).await,
			Command::Remove(cmd) => cmd.run(ctx).await,
			Command::List(cmd) => cmd.run(ctx).await,
			Command::Install(cmd) => cmd.run(ctx).await,
			Command::Outdated(cmd) => cmd.run(ctx).await,
			Command::Update(cmd) => cmd.run(ctx).await,
			Command::Run(cmd) => cmd.run(ctx).await,
			Command::Execute(cmd) => cmd.run(ctx).await,
		}
	}
}

#[derive(Debug)]
pub struct CommandContext {
	reqwest: std::sync::LazyLock<reqwest::Client>,
	cwd: std::sync::LazyLock<std::path::PathBuf>,
	project: tokio::sync::OnceCell<(Arc<FsVfs>, Project, WorkspaceMember)>,
	cli_config: tokio::sync::OnceCell<crate::cli::config::CliConfig>,
}

impl CommandContext {
	pub fn new() -> Self {
		Self {
			reqwest: std::sync::LazyLock::new(|| {
				reqwest::Client::builder()
					.user_agent(concat!(
						env!("CARGO_PKG_NAME"),
						"/",
						env!("CARGO_PKG_VERSION")
					))
					.build()
					.expect("failed to build reqwest client")
			}),
			cwd: std::sync::LazyLock::new(|| {
				std::env::current_dir().expect("failed to get current working directory")
			}),
			project: tokio::sync::OnceCell::new(),
			cli_config: tokio::sync::OnceCell::new(),
		}
	}

	pub fn reqwest(&self) -> &reqwest::Client {
		&self.reqwest
	}
}

mod impls {
	use crate::cli::config::read_config;

	use super::*;

	impl CommandContext {
		pub async fn vfs(&self) -> anyhow::Result<&Arc<FsVfs>> {
			Ok(&self.init_project().await?.0)
		}

		pub async fn project(&self) -> anyhow::Result<&Project> {
			Ok(&self.init_project().await?.1)
		}

		pub async fn member(&self) -> anyhow::Result<&WorkspaceMember> {
			Ok(&self.init_project().await?.2)
		}

		pub async fn execute_script<H: crate::cli::scripts::ExecuteScriptHooks>(
			&self,
			script: &str,
			hooks: &mut H,
			args: Vec<std::ffi::OsString>,
		) -> anyhow::Result<i32> {
			crate::cli::scripts::execute_script(self.cwd.clone(), script, hooks, args).await
		}

		pub async fn cli_config(&self) -> anyhow::Result<&crate::cli::config::CliConfig> {
			self.cli_config
				.get_or_try_init(|| async { read_config().await })
				.await
		}

		async fn init_project(&self) -> anyhow::Result<&(Arc<FsVfs>, Project, WorkspaceMember)> {
			self.project
				.get_or_try_init(|| async {
					let (vfs, importer) = FsVfs::in_project(self.cwd.clone())
						.await
						.context("failed to find project roots")?;

					tracing::debug!(
						"project root: `{}`, importer: `{importer}`, cas root: `{}`",
						vfs.project_dir.display(),
						vfs.cas_dir.display()
					);

					let config = self.cli_config().await?;

					let vfs = Arc::from(vfs);

					let project = pesde::Project::new(vfs.clone(), |workspace| {
						Arc::from(CliSourceProvider::new(
							workspace.clone(),
							self.reqwest().clone(),
							config.auth_scripts.clone(),
						))
					});

					let member = project.member(importer);
					Ok((vfs, project, member))
				})
				.await
		}
	}
}
