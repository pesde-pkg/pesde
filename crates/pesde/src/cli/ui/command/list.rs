use std::collections::BTreeMap;

use anyhow::Context as _;
use clap::Args;
use pesde::registry::manifest::Alias;
use pesde::registry::manifest::DependencyType;
use pesde::source::DependencySpecifier;

use crate::cli::ui::CommandContext;
use crate::cli::ui::dependency_type_to_key;

/// List all dependencies in the subproject
#[derive(Debug, Args)]
pub struct ListCommand;

impl ListCommand {
	pub async fn run(self, ctx: CommandContext) -> anyhow::Result<()> {
		let manifest = ctx
			.member()
			.await?
			.deser_manifest()
			.await
			.context("failed to read manifest")?;

		let all_deps = manifest.dependencies.iter().fold(
			BTreeMap::<DependencyType, BTreeMap<Alias, DependencySpecifier>>::new(),
			|mut acc, (alias, (spec, ty))| {
				acc.entry(*ty)
					.or_default()
					.insert(alias.clone(), spec.clone());
				acc
			},
		);

		for (dep_ty, deps) in all_deps {
			println!("{}:", dependency_type_to_key(dep_ty));

			for (alias, spec) in deps {
				println!("{alias}: {spec}");
			}

			println!();
		}

		Ok(())
	}
}
