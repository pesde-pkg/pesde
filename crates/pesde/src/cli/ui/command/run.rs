use anyhow::Context as _;
use clap::Args;
use std::ffi::OsString;

use crate::cli::ui::CommandContext;

/// Executes a script with pesde's shell environment
#[derive(Debug, Args)]
pub struct RunCommand {
	/// Interpret the script as a raw command instead of looking it up in the manifest
	#[arg(short, long)]
	raw: bool,

	/// The script to run
	script: Box<str>,

	/// Arguments to pass to the script
	#[arg(allow_hyphen_values = true)]
	args: Vec<OsString>,
}

impl RunCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let manifest = if self.raw {
			None
		} else {
			let manifest = ctx
				.member()
				.await?
				.deser_manifest()
				.await
				.context("failed to deserialize manifest")?;

			Some(manifest)
		};

		let script = if let Some(manifest) = &manifest {
			manifest
				.scripts
				.get(&self.script)
				.context("script not found")?
		} else {
			&*self.script
		};

		let code = ctx.execute_script(script, &mut (), self.args).await?;
		std::process::exit(code);
	}
}
