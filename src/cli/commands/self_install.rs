use crate::cli::{
	style::{ADDED_STYLE, CLI_STYLE},
	version::replace_pesde_bin_exe,
};
use anyhow::Context as _;
use clap::Args;
use console::style;
use std::env::current_exe;

#[derive(Debug, Args)]
pub struct SelfInstallCommand {
	/// Skip adding the bin directory to the PATH
	#[cfg(windows)]
	#[arg(short, long)]
	skip_add_to_path: bool,
}

impl SelfInstallCommand {
	pub async fn run(self) -> anyhow::Result<()> {
		let bin_dir = crate::cli::bin_dir()?;
		let bin_dir = bin_dir
			.to_str()
			.context("bin directory path contains invalid characters")?;

		#[cfg(windows)]
		{
			if !self.skip_add_to_path {
				use crate::cli::style::WARN_STYLE;
				use anyhow::Context as _;
				use windows_registry::CURRENT_USER;

				let env = CURRENT_USER
					.create("Environment")
					.context("failed to open Environment key")?;
				let path = env.get_string("Path").context("failed to get Path value")?;

				let exists = path.split(';').any(|part| part == bin_dir);

				if !exists {
					let new_path = format!("{path};{bin_dir}");
					env.set_string("Path", &new_path)
						.context("failed to set Path value")?;

					println!(
						"\nin order to allow proper functionality {} was added to PATH.\n\n{}",
						style(format!("`{bin_dir}`")).green(),
						WARN_STYLE.apply_to("please restart your shell for this to take effect")
					);
				}
			}

			println!(
				"installed {} {}!",
				CLI_STYLE.apply_to(env!("CARGO_BIN_NAME")),
				ADDED_STYLE.apply_to(env!("CARGO_PKG_VERSION")),
			);
		};

		#[cfg(unix)]
		{
			println!(
				r"installed {} {}! add the following line to your shell profile in order to get the binary and binary exports as executables usable from anywhere:

{}

and then restart your shell.
",
				CLI_STYLE.apply_to(env!("CARGO_BIN_NAME")),
				ADDED_STYLE.apply_to(env!("CARGO_PKG_VERSION")),
				style(format!(r#"export PATH="$PATH:{bin_dir}""#)).green(),
			);
		};

		replace_pesde_bin_exe(&current_exe().context("failed to get current exe path")?).await?;

		Ok(())
	}
}
