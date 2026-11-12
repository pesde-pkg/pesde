use std::str::FromStr as _;

use anyhow::Context as _;
use clap::Args;
use pesde::registry::manifest::Alias;
use pesde::registry::manifest::DependencyType;

use crate::cli::ui::CommandContext;
use crate::cli::ui::dependency_type_to_key;

/// Removes a package from the manifest
#[derive(Debug, Args)]
pub struct RemoveCommand {
	/// The alias of the package to remove
	alias: Alias,
}

impl RemoveCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let member = ctx.member().await?;

		let mut manifest = toml_edit::DocumentMut::from_str(
			&member
				.read_manifest()
				.await
				.context("failed to read manifest")?,
		)
		.context("failed to parse manifest")?;

		let Some(dep_key) = [
			DependencyType::Standard,
			DependencyType::Dev,
			DependencyType::Peer,
		]
		.into_iter()
		.map(dependency_type_to_key)
		.find(|dependency_key| {
			manifest[dependency_key]
				.as_table_mut()
				.is_some_and(|table| table.remove(self.alias.as_str()).is_some())
		}) else {
			anyhow::bail!("package under alias `{}` not found in manifest", self.alias)
		};

		member
			.write_manifest(manifest.to_string())
			.await
			.context("failed to write manifest")?;

		println!("removed `{}` from `{dep_key}`!", self.alias);

		Ok(())
	}
}
