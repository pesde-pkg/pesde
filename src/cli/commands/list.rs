use std::collections::BTreeMap;

use anyhow::Context as _;
use clap::Args;

use crate::cli::{
	dep_type_to_key,
	style::{INFO_STYLE, SUCCESS_STYLE},
};
use pesde::{
	Subproject,
	manifest::{Alias, DependencyType},
	source::specifiers::DependencySpecifiers,
};

#[derive(Debug, Args)]
pub struct ListCommand;

impl ListCommand {
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		let manifest = subproject
			.deser_manifest()
			.await
			.context("failed to read manifest")?;

		let all_deps = manifest
			.all_dependencies()
			.context("failed to get all dependencies")?
			.into_iter()
			.fold(
				BTreeMap::<DependencyType, BTreeMap<Alias, DependencySpecifiers>>::new(),
				|mut acc, (alias, (spec, ty))| {
					acc.entry(ty).or_default().insert(alias, spec);
					acc
				},
			);

		for (dep_ty, deps) in all_deps {
			let dep_key = dep_type_to_key(dep_ty);
			println!("{}", INFO_STYLE.apply_to(dep_key));

			for (alias, spec) in deps {
				println!("{}: {spec}", SUCCESS_STYLE.apply_to(alias));
			}

			println!();
		}

		Ok(())
	}
}
