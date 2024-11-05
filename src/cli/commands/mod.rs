use indicatif::MultiProgress;
use pesde::Project;

mod add;
mod auth;
mod config;
mod execute;
mod init;
mod install;
mod outdated;
#[cfg(feature = "patches")]
mod patch;
#[cfg(feature = "patches")]
mod patch_commit;
mod publish;
mod run;
#[cfg(feature = "version-management")]
mod self_install;
#[cfg(feature = "version-management")]
mod self_upgrade;
mod update;

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    /// Authentication-related commands
    Auth(auth::AuthSubcommand),

    /// Configuration-related commands
    #[command(subcommand)]
    Config(config::ConfigCommands),

    /// Initializes a manifest file in the current directory
    Init(init::InitCommand),

    /// Runs a script, an executable package, or a file with Lune
    Run(run::RunCommand),

    /// Installs all dependencies for the project
    Install(install::InstallCommand),

    /// Publishes the project to the registry
    Publish(publish::PublishCommand),

    /// Installs the pesde binary and scripts
    #[cfg(feature = "version-management")]
    SelfInstall(self_install::SelfInstallCommand),

    /// Sets up a patching environment for a package
    #[cfg(feature = "patches")]
    Patch(patch::PatchCommand),

    /// Finalizes a patching environment for a package
    #[cfg(feature = "patches")]
    PatchCommit(patch_commit::PatchCommitCommand),

    /// Installs the latest version of pesde
    #[cfg(feature = "version-management")]
    SelfUpgrade(self_upgrade::SelfUpgradeCommand),

    /// Adds a dependency to the project
    Add(add::AddCommand),

    /// Updates the project's lockfile. Run install to apply changes
    Update(update::UpdateCommand),

    /// Checks for outdated dependencies
    Outdated(outdated::OutdatedCommand),

    /// Executes a binary package without needing to be run in a project directory
    #[clap(name = "x", visible_alias = "execute", visible_alias = "exec")]
    Execute(execute::ExecuteCommand),
}

impl Subcommand {
    pub async fn run(
        self,
        project: Project,
        multi: MultiProgress,
        reqwest: reqwest::Client,
    ) -> anyhow::Result<()> {
        match self {
            Subcommand::Auth(auth) => auth.run(project, reqwest).await,
            Subcommand::Config(config) => config.run().await,
            Subcommand::Init(init) => init.run(project).await,
            Subcommand::Run(run) => run.run(project).await,
            Subcommand::Install(install) => install.run(project, multi, reqwest).await,
            Subcommand::Publish(publish) => publish.run(project, reqwest).await,
            #[cfg(feature = "version-management")]
            Subcommand::SelfInstall(self_install) => self_install.run().await,
            #[cfg(feature = "patches")]
            Subcommand::Patch(patch) => patch.run(project, reqwest).await,
            #[cfg(feature = "patches")]
            Subcommand::PatchCommit(patch_commit) => patch_commit.run(project).await,
            #[cfg(feature = "version-management")]
            Subcommand::SelfUpgrade(self_upgrade) => self_upgrade.run(reqwest).await,
            Subcommand::Add(add) => add.run(project).await,
            Subcommand::Update(update) => update.run(project, multi, reqwest).await,
            Subcommand::Outdated(outdated) => outdated.run(project).await,
            Subcommand::Execute(execute) => execute.run(project, reqwest).await,
        }
    }
}
