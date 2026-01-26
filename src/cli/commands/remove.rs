use std::str::FromStr as _;

use anyhow::Context as _;
use clap::Args;

use crate::cli::{
	dep_type_to_key,
	style::{INFO_STYLE, SUCCESS_STYLE},
};
use pesde::{
	Subproject,
	manifest::{Alias, DependencyType},
};

#[derive(Debug, Args)]
pub struct RemoveCommand {
	/// The alias of the package to remove
	#[arg(index = 1)]
	alias: Alias,
}

impl RemoveCommand {
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		let mut manifest = toml_edit::DocumentMut::from_str(
			&subproject
				.read_manifest()
				.await
				.context("failed to read manifest")?,
		)
		.context("failed to parse manifest")?;

		let Some(dep_key) = DependencyType::VARIANTS
			.iter()
			.copied()
			.map(dep_type_to_key)
			.find(|dependency_key| {
				manifest[dependency_key]
					.as_table_mut()
					.is_some_and(|table| table.remove(self.alias.as_str()).is_some())
			})
		else {
			anyhow::bail!("package under alias `{}` not found in manifest", self.alias)
		};

		subproject
			.write_manifest(manifest.to_string())
			.await
			.context("failed to write manifest")?;

		println!(
			"{} removed {} from {}!",
			SUCCESS_STYLE.apply_to("success!"),
			INFO_STYLE.apply_to(self.alias),
			INFO_STYLE.apply_to(dep_key)
		);

		Ok(())
	}
}
