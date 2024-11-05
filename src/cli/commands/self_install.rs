use crate::cli::{version::update_bin_exe, HOME_DIR};
use clap::Args;
use colored::Colorize;
#[derive(Debug, Args)]
pub struct SelfInstallCommand {
    /// Skip adding the bin directory to the PATH
    #[cfg(windows)]
    #[arg(short, long)]
    skip_add_to_path: bool,
}

impl SelfInstallCommand {
    pub async fn run(self) -> anyhow::Result<()> {
        #[cfg(windows)]
        {
            if !self.skip_add_to_path {
                use anyhow::Context;
                use winreg::{enums::HKEY_CURRENT_USER, RegKey};

                let current_user = RegKey::predef(HKEY_CURRENT_USER);
                let env = current_user
                    .create_subkey("Environment")
                    .context("failed to open Environment key")?
                    .0;
                let path: String = env.get_value("Path").context("failed to get Path value")?;

                let bin_dir = crate::cli::bin_dir().await?;
                let bin_dir = bin_dir.to_string_lossy();

                let exists = path.split(';').any(|part| *part == bin_dir);

                if !exists {
                    let new_path = format!("{path};{bin_dir}");
                    env.set_value("Path", &new_path)
                        .context("failed to set Path value")?;

                    println!(
                        "\nin order to allow binary exports as executables {}.\n\n{}",
                        format!("`~/{HOME_DIR}/bin` was added to PATH").green(),
                        "please restart your shell for this to take effect"
                            .yellow()
                            .bold()
                    );
                }
            }

            println!(
                "installed {} {}!",
                env!("CARGO_BIN_NAME").cyan(),
                env!("CARGO_PKG_VERSION").yellow(),
            );
        }

        #[cfg(unix)]
        {
            println!(
                r#"installed {} {}! add the following line to your shell profile in order to get the binary and binary exports as executables usable from anywhere:

{}

and then restart your shell.
"#,
                env!("CARGO_BIN_NAME").cyan(),
                env!("CARGO_PKG_VERSION").yellow(),
                format!(r#"export PATH="$PATH:~/{HOME_DIR}/bin""#)
                    .bold()
                    .green()
            );
        }

        update_bin_exe().await?;

        Ok(())
    }
}
