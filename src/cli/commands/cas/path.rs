use clap::Args;
use pesde::Subproject;

#[derive(Debug, Args)]
pub struct PathCommand;

impl PathCommand {
	pub fn run(self, subproject: &Subproject) {
		println!("{}", subproject.project().cas_dir().display());
	}
}
