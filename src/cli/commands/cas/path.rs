use clap::Args;
use pesde::Project;

#[derive(Debug, Args)]
pub struct PathCommand;

impl PathCommand {
	#[allow(clippy::unnecessary_wraps, clippy::needless_pass_by_value)]
	pub fn run(self, project: Project) -> anyhow::Result<()> {
		println!("{}", project.cas_dir().display());

		Ok(())
	}
}
