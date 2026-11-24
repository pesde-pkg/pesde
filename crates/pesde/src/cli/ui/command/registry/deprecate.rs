use anyhow::Context as _;
use clap::Args;
use pesde::bounded::BoundedStr;
use pesde::hash::Hash;
use pesde::registry::*;

use crate::cli::registry::TreeUpdate;
use crate::cli::ui::command::registry::RegistryCommandContext;
use crate::cli::ui::public_key::UserInputPublicKey;
use crate::cli::ui::signature::SignatureSourceArgs;

/// Manages a package's yank status in the registry
#[derive(Debug, Args)]
pub struct DeprecateCommand {
	/// The package name to deprecate
	package_name: PackageName,

	/// The reason of deprecation, if empty, the package will be un-deprecated
	#[arg(default_value = "")]
	reason: BoundedStr<MAX_REASON_LEN>,

	/// The public key to use for signing the package. If not specified, the associated key of the registry will be used
	#[arg(short, long)]
	public_key: Option<UserInputPublicKey>,

	#[command(flatten)]
	source: SignatureSourceArgs,
}

impl DeprecateCommand {
	pub async fn run(self, ctx: RegistryCommandContext) -> anyhow::Result<()> {
		eprintln!(
			"WARNING: you are about to push an operation to the registry that will deprecate the following package: {}",
			self.package_name,
		);

		eprintln!();

		ctx.signature_lecture()?;

		let client = ctx.client();

		let scope_id = ScopeId::from(self.package_name.scope());
		let pkg_id = LocalNameId::from(self.package_name.local_name());

		let tip_entry = ctx
			.client()
			.scope_head(&scope_id)
			.await?
			.context("scope does not exist")?
			.tip
			.entry;
		let mut scope_state = tip_entry.payload.header().post_state.clone();

		let key = PackageDeprecation(pkg_id);

		let (update, value) = if self.reason.is_empty() {
			(TreeUpdate::Remove { key }, None)
		} else {
			let value = PackageDeprecationValue {
				reason: PackageDeprecationReason::new(
					Hash::digest(Default::default(), self.reason.as_bytes()),
					self.reason,
				)
				.context("failed to validate deprecation reason")?,
			};

			(
				TreeUpdate::Insert {
					key,
					value: value.clone(),
				},
				Some(value),
			)
		};

		scope_state.package_deprecations_root = client
			.tree_transition(&scope_state.package_deprecations_root, [update])
			.await?;

		let body = SetDeprecationUserScopeOpBody {
			header: UserScopeOpHeader {
				signer: scope_state.owner.clone(),
				common: ScopeOpHeader {
					scope_id,
					prev_hash: tip_entry.hash(),
					post_state: scope_state,
				},
			},
			pkg: pkg_id,
			state: value,
		};

		let body = self
			.source
			.signed(&ctx, UserScopeOp::SetDeprecation(body))
			.await?;
		let body = ScopeEntryPayload::User(body);

		let response = client.post_scope_entry(&body).await?;

		println!(
			"successfully set deprecation status for package {}. server operation timestamp: {}",
			self.package_name, response.published_at
		);

		Ok(())
	}
}
