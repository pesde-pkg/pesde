use anyhow::Context as _;
use clap::Subcommand;
use pesde::registry::AddMemberUserScopeOpBody;
use pesde::registry::Consent;
use pesde::registry::Entry;
use pesde::registry::LogHeadResponse;
use pesde::registry::ScopeEntryPayload;
use pesde::registry::ScopeId;
use pesde::registry::ScopeMember;
use pesde::registry::ScopeMemberValue;
use pesde::registry::ScopeMembersTree;
use pesde::registry::ScopeOpHeader;
use pesde::registry::Signable as _;
use pesde::registry::Signed;
use pesde::registry::TreeOperation;
use pesde::registry::TreeTransitionProof;
use pesde::registry::TreeTransitionResult;
use pesde::registry::UnvalidatedConsent;
use pesde::registry::UnvalidatedSigned;
use pesde::registry::UserScopeOp;
use pesde::registry::UserScopeOpHeader;
use reqwest::Method;
use reqwest::header::AUTHORIZATION;

use crate::cli::rootless_url;
use crate::cli::ui::command::scope::ScopeCommandContext;
use crate::cli::ui::print_immutable_protocol_warning;
use crate::cli::ui::public_key::UserInputPublicKey;
use crate::cli::ui::signature::SignatureSourceArgs;
use crate::cli::url_component_encode;

/// Member addition
#[derive(Debug, Subcommand)]
pub enum AddMemberCommand {
	/// Creates a consent message for the member to send to the owner of the scope
	///
	/// The message is printed to stdout and must be delivered to the owner out-of-band
	Create {
		/// The public key to add as a member of the scope
		public_key: UserInputPublicKey,

		#[command(flatten)]
		source: SignatureSourceArgs,
	},

	/// Prints out information about the member being added and pushes the operation to the registry
	Apply {
		/// The consent message to sign
		message: Box<str>,

		/// Whether to sign the consent message without prompting for confirmation
		#[arg(long)]
		no_prompt: bool,

		#[command(flatten)]
		source: SignatureSourceArgs,
	},
}

impl AddMemberCommand {
	pub async fn run(self, ctx: ScopeCommandContext) -> anyhow::Result<()> {
		match self {
			AddMemberCommand::Create { public_key, source } => {
				let source = source.get_source(&ctx, &public_key.key).await?;

				eprintln!(
					"WARNING: you are about to sign a message that will allow the owner of the scope to add you as a member"
				);
				eprintln!("you may be liable for any actions taken by the scope's management");

				eprintln!();

				print_immutable_protocol_warning();

				eprintln!();

				let scope_id = ScopeId::from(ctx.scope());

				let consent = UnvalidatedConsent {
					// the nonce is used only to prevent replaying this signature, it doesn't need to be securely random
					nonce: rand::random(),
					consenter: public_key.key,
				};
				let message =
					Consent::<AddMemberUserScopeOpBody>::signing_bytes(&scope_id, &consent);

				let sig = source
					.sign(&consent.consenter, &message)
					.await
					.context("failed to sign message")?;

				let consent = Consent::<AddMemberUserScopeOpBody>::new(
					&scope_id,
					UnvalidatedSigned { sig, body: consent },
				)
				.context("failed to validate signature")?;

				print!(
					"{}",
					fast32::base64::RFC4648_NOPAD.encode(&serde_json::to_vec(&consent)?)
				);
			}
			AddMemberCommand::Apply {
				message,
				no_prompt,
				source,
			} => {
				let consent = fast32::base64::RFC4648_NOPAD
					.decode_str(&message)
					.context("malformed message")?;

				let consent: UnvalidatedSigned<UnvalidatedConsent> =
					serde_json::from_slice(&consent).context("failed to parse message")?;

				let scope_id = ScopeId::from(ctx.scope());

				let consent = Consent::<AddMemberUserScopeOpBody>::new(&scope_id, consent)
					// check to avoid sending redundant requests
					.context("consent was not for the expected scope or had a malformed signature")?
					.into_inner();

				// TODO: prompt for this
				let grant = pesde::registry::ScopeGrant(Default::default());

				eprintln!("scope: {}", ctx.scope());
				eprintln!("member public key: {}", consent.body.consenter);
				eprintln!(
					"member nonce: {}",
					fast32::base32::CROCKFORD_LOWER.encode(&consent.body.nonce)
				);

				eprintln!(
					"WARNING: you are about to push an operation to the registry that will add a member to the scope"
				);

				eprintln!();

				print_immutable_protocol_warning();

				eprintln!();

				if !no_prompt {
					inquire::Confirm::new("do you want to continue?")
						.with_default(false)
						.with_help_message("this action cannot be undone")
						.prompt()
						.context("failed to read user input")?
						.ok_or_else(|| anyhow::anyhow!("aborted by user"))?;
				}

				let authorization = ctx.authorization().await?;

				let mut head_request = ctx.reqwest().get(format!(
					"{}/v2/api/scope/{}/log/head",
					rootless_url(ctx.registry()),
					url_component_encode(&scope_id.to_string())
				));

				if let Some(auth) = authorization {
					head_request = head_request.header(AUTHORIZATION, auth);
				}

				// TODO: check transparency log consistency & inclusion proofs
				let head: LogHeadResponse<ScopeEntryPayload> = head_request
					.send()
					.await
					.context("failed to send request to registry")?
					.error_for_status()
					.context("registry returned an error response")?
					.json()
					.await
					.context("failed to parse response from registry")?;
				let tip_entry = head.tip.entry;

				let mut scope_state = tip_entry.payload.header().post_state.clone();

				let tree_key = ScopeMember(consent.body.consenter.clone());
				let tree_value = ScopeMemberValue {
					grant: grant.clone(),
				};

				let mut transition_request = ctx
					.reqwest()
					.request(
						Method::QUERY,
						format!(
							"{}/v2/api/tree/scope_members/{}/transition",
							rootless_url(ctx.registry()),
							url_component_encode(&scope_state.scope_members_root.0.to_string()),
						),
					)
					.json(&[TreeOperation::<ScopeMembersTree>::Modify {
						key: tree_key.clone(),
						value: tree_value.clone(),
					}]);

				if let Some(auth) = authorization {
					transition_request = transition_request.header(AUTHORIZATION, auth);
				}

				let [transition_response] = transition_request
					.send()
					.await
					.context("failed to send request to registry")?
					.error_for_status()
					.context("registry returned an error response")?
					.json::<[TreeTransitionResult<ScopeMembersTree>; 1]>()
					.await
					.context("failed to parse response from registry")?;

				let TreeTransitionProof::Modify(proof) = transition_response.proof else {
					anyhow::bail!("registry returned an unexpected proof type");
				};

				let verify_result = proof.verify(
					&scope_state.scope_members_root.0,
					&transition_response.root.0,
					&tree_key,
					&tree_value,
					&Default::default(),
				);

				match verify_result {
					Ok(true) => {}
					Ok(false) => anyhow::bail!("identity already member of scope"),
					Err(_) => {
						anyhow::bail!("failed to verify proof of tree transition from registry")
					}
				}

				scope_state.scope_members_root = transition_response.root;

				let body = AddMemberUserScopeOpBody::new(
					UserScopeOpHeader {
						signer: scope_state.owner.clone(),
						common: ScopeOpHeader {
							scope_id: scope_id.clone(),
							prev_hash: tip_entry.hash(),
							post_state: scope_state,
						},
					},
					consent,
					pesde::registry::ScopeGrant(Default::default()),
				)
				.context("failed to construct add member operation")?;

				let sig = source
					.get_source(&ctx, &body.header().signer)
					.await?
					.sign(&body.header().signer, &body.signing_bytes())
					.await
					.context("failed to sign add member operation")?;

				let body = UserScopeOp::AddMember(body);
				let body = UnvalidatedSigned { sig, body };
				let body = Signed::new(body).context("failed to validate signature")?;
				let body = ScopeEntryPayload::User(body);

				let mut request = ctx
					.reqwest()
					.post(format!(
						"{}/v2/api/scope/log/entry",
						rootless_url(ctx.registry()),
					))
					.json(&body);

				if let Some(auth) = ctx.authorization().await? {
					request = request.header(AUTHORIZATION, auth);
				}

				let response: Entry<ScopeEntryPayload> = request
					.send()
					.await
					.context("failed to send request to registry")?
					.error_for_status()
					.context("registry returned an error response")?
					.json()
					.await
					.context("failed to parse response from registry")?;

				println!(
					"successfully added member to scope. server operation timestamp: {}",
					response.published_at
				);
			}
		}

		Ok(())
	}
}
