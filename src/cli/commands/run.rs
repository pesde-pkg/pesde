use anyhow::Context as _;
use clap::Args;
use pesde::Subproject;
use std::ffi::OsString;

#[derive(Debug, Args)]
pub struct RunCommand {
	/// The script name to run
	#[arg(index = 1)]
	script: String,

	/// Arguments to pass to the script
	#[arg(index = 2, last = true)]
	args: Vec<OsString>,
}

impl RunCommand {
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		let manifest = subproject
			.deser_manifest()
			.await
			.context("failed to deserialize manifest")?;
		pesde::scripts::execute_script(
			&subproject,
			manifest
				.scripts
				.get(&self.script)
				.context("script not found")?,
			&mut (),
			self.args,
		)
		.await?;
		Ok(())
	}
}
