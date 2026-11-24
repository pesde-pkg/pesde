use std::collections::BTreeSet;
use std::str::FromStr as _;

use anyhow::Context as _;
use clap::Args;
use inquire::validator::Validation;
use pesde::bounded::BoundedBTreeSet;
use pesde::registry::LocalName;
use pesde::registry::LocalNameId;
use pesde::registry::ScopeGrant;
use pesde::registry::ScopeGrantEntry;

#[derive(Debug, Args)]
pub struct ScopeGrantArgs {
	/// A comma-separated list of package names to give the member management of
	///
	/// WARNING: an empty value will give the member management of all packages in the scope.
	/// For this reason, this option is not recommended as a prompt will be displayed
	/// to interactively prompt for package names if this option is not specified,
	/// letting the user choose with more care
	#[arg(long, hide_short_help = true)]
	grant: Option<Box<str>>,
}

impl ScopeGrantArgs {
	pub fn get(self) -> anyhow::Result<ScopeGrant> {
		let mut grant = BTreeSet::<ScopeGrantEntry>::new();
		let mk_grant =
			|name: LocalName| ScopeGrantEntry::new(LocalNameId::from(&name), Some(name)).unwrap();

		if let Some(grant_str) = self.grant {
			for name in grant_str.split(',') {
				grant.insert(mk_grant(
					LocalName::from_str(name).context("invalid package name")?,
				));
			}

			return Ok(ScopeGrant(
				BoundedBTreeSet::new(grant).context("too many package names")?,
			));
		}

		let mut grant = BTreeSet::new();
		let all = inquire::Confirm::new(
			"do you want to give the member management of all packages in the scope?",
		)
		.with_default(false)
		.prompt()
		.context("failed to read input")?;

		if !all {
			loop {
				let text =
					inquire::Text::new("enter a package name to give the member management of")
						.with_validator(|input: &str| {
							if input.is_empty() {
								return Ok(Validation::Valid);
							}

							Ok(match LocalName::from_str(input) {
								Ok(_) => Validation::Valid,
								Err(e) => Validation::Invalid(e.to_string().into()),
							})
						})
						.prompt()
						.context("failed to read input")?;

				if text.is_empty() {
					break;
				}

				grant.insert(mk_grant(LocalName::from_str(&text).unwrap()));
			}
		}

		Ok(ScopeGrant(
			BoundedBTreeSet::new(grant).context("too many package names")?,
		))
	}
}
