use std::str::FromStr;

use anyhow::Context as _;
use clap::Args;
use pesde::registry::*;

use crate::cli::registry::TreeUpdate;
use crate::cli::ui::command::registry::RegistryCommandContext;
use crate::cli::ui::public_key::UserInputPublicKey;
use crate::cli::ui::signature::SignatureSourceArgs;

/// Manages a package version's yank status in the registry
#[derive(Debug, Args)]
pub struct YankCommand {
	/// A package name and an optional version to yank. If no version is specified, the current directory's package name and version will be used
	package: UserInputPackageVersion,

	/// Whether to un-yank the package version instead of yanking it
	#[arg(long)]
	undo: bool,

	/// The public key to use for signing the package. If not specified, the associated key of the registry will be used
	#[arg(short, long)]
	public_key: Option<UserInputPublicKey>,

	#[command(flatten)]
	source: SignatureSourceArgs,
}

impl YankCommand {
	pub async fn run(self, ctx: RegistryCommandContext) -> anyhow::Result<()> {
		let version = if let Some(version) = self.package.version {
			version
		} else {
			let manifest = ctx.member().await?.deser_manifest().await?;
			if manifest.package.name != self.package.package_name {
				anyhow::bail!(
					"no versions specified and package name `{}` does not match the current directory's package name `{}`",
					self.package.package_name,
					manifest.package.name
				);
			}

			manifest.package.version.clone()
		};

		eprintln!(
			"WARNING: you are about to push an operation to the registry that will yank the following version of package `{}`: {version}",
			self.package.package_name,
		);

		eprintln!();

		ctx.signature_lecture()?;

		let client = ctx.client();

		let scope_id = ScopeId::from(self.package.package_name.scope());
		let pkg_id = LocalNameId::from(self.package.package_name.local_name());

		let tip_entry = ctx
			.client()
			.scope_head(&scope_id)
			.await?
			.context("scope does not exist")?
			.tip
			.entry;
		let mut scope_state = tip_entry.payload.header().post_state.clone();

		let key = PackageVersion {
			pkg: pkg_id,
			version: version.clone(),
		};
		let package_value = client
			.tree_entry(&scope_state.package_versions_root, &key)
			.await?;
		let Some(mut package_value) = package_value else {
			anyhow::bail!("package version does not exist in the registry");
		};

		package_value.yank_state = match package_value.yank_state {
			Some(VersionYankState::AdminYanked) => anyhow::bail!(
				"package version is yanked by the registry administrator and cannot be un-yanked"
			),
			Some(VersionYankState::Yanked) => {
				if !self.undo {
					anyhow::bail!("package version is already yanked");
				}

				None
			}
			None => {
				if self.undo {
					anyhow::bail!("package version is not yanked");
				}

				Some(VersionYankState::Yanked)
			}
		};
		let new_yanked = package_value.yank_state.is_some();

		scope_state.package_versions_root = client
			.tree_transition(
				&scope_state.package_versions_root,
				[TreeUpdate::Update {
					key: key.clone(),
					value: package_value,
				}],
			)
			.await?;

		let body = SetYankedUserScopeOpBody {
			header: UserScopeOpHeader {
				signer: scope_state.owner.clone(),
				common: ScopeOpHeader {
					scope_id,
					prev_hash: tip_entry.hash(),
					post_state: scope_state,
				},
			},
			package_version: key.clone(),
			yanked: new_yanked,
		};

		let body = self
			.source
			.signed(&ctx, UserScopeOp::SetYanked(body))
			.await?;
		let body = ScopeEntryPayload::User(body);

		let response = client.post_scope_entry(&body).await?;

		println!(
			"successfully set yanked status for package version {}@{}. server operation timestamp: {}",
			self.package.package_name, key.version, response.published_at
		);

		Ok(())
	}
}

#[derive(Debug, Clone)]
struct UserInputPackageVersion {
	package_name: PackageName,
	version: Option<PesdeStyleVersion>,
}

impl FromStr for UserInputPackageVersion {
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let mut parts = s.splitn(2, '@');
		let package_name = parts
			.next()
			.context("package name is required")?
			.parse::<PackageName>()?;

		Ok(Self {
			package_name,
			version: parts.next().map(str::parse).transpose()?,
		})
	}
}
