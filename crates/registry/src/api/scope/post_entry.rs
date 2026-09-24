use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::post;
use actix_web::web;
use merkle_bplustree::InsertNewError;
use merkle_bplustree::MerkleBPlusTree;
use merkleberg::MMRIVER;
use pesde::hash::Hash;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::db::ExistingScopeLockResult;
use pesde_registry_core::db::PermissionWidth;

use crate::AppState;
use crate::api::scope::error::Error;
use crate::shared::db::run_tx;

#[post("/scope/log/entry")]
pub(super) async fn http_v2(
	app_state: web::Data<AppState>,
	body: web::Json<Signed<UserScopeOp>>,
) -> Result<impl Responder, Error> {
	let entry = handler(app_state.db.as_ref(), body.into_inner()).await?;

	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(db: &dyn Backend, payload: Signed<UserScopeOp>) -> Result<ScopeEntry, Error> {
	let payload = payload.inner();

	let UnvalidatedSigned { sig: _, body: op } = &payload;
	let UserScopeOpHeader {
		common: ScopeOpHeader {
			scope_id,
			prev_hash: given_prev_hash,
		},
		signer,
	} = op.header();

	let permissions = match op {
		UserScopeOp::AddMember(AddMemberUserScopeOpBody {
			kind: _,
			header: _,
			member,
			grant: _,
			scope_members_root: _,
		}) => {
			if member.consenter() == signer {
				return Err(Error::OwnerAsMember);
			}

			PermissionWidth::Owner
		}
		UserScopeOp::UpdateMemberGrant(UpdateMemberGrantUserScopeOpBody {
			kind: _,
			header: _,
			member,
			grant: _,
			scope_members_root: _,
		}) => {
			if member == signer {
				return Err(Error::OwnerAsMember);
			}

			PermissionWidth::Owner
		}
		UserScopeOp::RotateKey(RotateKeyUserScopeOpBody {
			kind: _,
			header: _,
			new_key,
			scope_members_root: _,
		}) => {
			if new_key.consenter() == signer {
				return Err(Error::KeyChangeNoChange);
			}

			PermissionWidth::OnlySelf
		}
		UserScopeOp::RemoveMember(RemoveMemberUserScopeOpBody {
			kind: _,
			header: _,
			member,
			scope_members_root: _,
		}) => match member {
			Some(k) => {
				if k == signer {
					return Err(Error::OwnerAsMember);
				}

				PermissionWidth::Owner
			}
			None => PermissionWidth::OnlySelf,
		},
		UserScopeOp::TransferOwnership(TransferOwnershipUserScopeOpBody {
			kind: _,
			header: _,
			new_owner,
		}) => {
			if signer == new_owner.consenter() {
				return Err(Error::KeyChangeNoChange);
			}

			PermissionWidth::Owner
		}
		UserScopeOp::PublishVersion(_) => {
			return Err(Error::PublishVersionInPostEntry);
		}
		UserScopeOp::SetYanked(op) => PermissionWidth::Package(&op.pkg),
		UserScopeOp::SetDeprecation(op) => PermissionWidth::Package(&op.pkg),
	};

	let (tx, scope_size) = match db
		.begin_write_existing(scope_id, signer, permissions)
		.await?
	{
		ExistingScopeLockResult::Ok { tx, scope_size } => (tx, scope_size),
		ExistingScopeLockResult::DoesntExist => return Err(Error::ScopeNotFound),
		ExistingScopeLockResult::Unauthorized => return Err(Error::Unauthorized),
	};

	run_tx(tx, async |tx| {
		let prev_entry = tx
			.scope_log_entry(scope_id, scope_size.get())
			.await?
			.ok_or(Error::ScopeNotFound)?;

		let prev_entry_hash =
			Hash::digest(given_prev_hash.algorithm(), canonical_bytes(&prev_entry));
		if *given_prev_hash != prev_entry_hash {
			return Err(Error::InvalidPrevHash);
		}

		let scope_state = async || {
			tx.scope_state(scope_id, scope_size)
				.await?
				.ok_or(Error::ScopeNotFound)
		};

		match op {
			UserScopeOp::AddMember(AddMemberUserScopeOpBody {
				kind: _,
				header: _,
				member,
				grant,
				scope_members_root,
			}) => {
				let state = scope_state().await?;
				let scope_members = MerkleBPlusTree::<ScopeMembersTree, _>::from_root_hash(
					&state.scope_members_root,
					todo!(),
					Default::default(),
				)
				.await?;

				match scope_members
					.insert_new(member.consenter().clone(), grant.clone())
					.await
				{
					Ok(_) => {}
					Err(InsertNewError::AlreadyExists) => return Err(Error::AlreadyExists),
					Err(InsertNewError::StorageError(e)) => return Err(e),
				}

				if scope_members.root_hash() != scope_members_root.0 {
					return Err(Error::ComputedRootDifferent);
				}
			}
			UserScopeOp::UpdateMemberGrant(UpdateMemberGrantUserScopeOpBody {
				kind: _,
				header: _,
				member,
				grant,
				scope_members_root,
			}) => {
				let state = scope_state().await?;
				let scope_members = MerkleBPlusTree::<ScopeMembersTree, _>::from_root_hash(
					&state.scope_members_root,
					todo!(),
					Default::default(),
				)
				.await?;

				let old = scope_members.insert(member.clone(), grant.clone()).await?;
				if old.is_none_or(|g| g == *grant) {
					return Err(Error::NotInScope);
				}

				if scope_members.root_hash() != scope_members_root.0 {
					return Err(Error::ComputedRootDifferent);
				}
			}
			UserScopeOp::RotateKey(RotateKeyUserScopeOpBody {
				kind: _,
				header: _,
				new_key,
				scope_members_root,
			}) => {
				let state = scope_state().await?;
				let scope_members = MerkleBPlusTree::<ScopeMembersTree, _>::from_root_hash(
					&state.scope_members_root,
					todo!(),
					Default::default(),
				)
				.await?;

				let Some(grant) = scope_members.delete(new_key.consenter()).await? else {
					return Err(Error::Unauthorized);
				};

				match scope_members
					.insert_new(new_key.consenter().clone(), grant)
					.await
				{
					Ok(_) => {}
					Err(InsertNewError::AlreadyExists) => return Err(Error::AlreadyExists),
					Err(InsertNewError::StorageError(e)) => return Err(e),
				}

				if scope_members.root_hash() != scope_members_root.0 {
					return Err(Error::ComputedRootDifferent);
				}
			}
			UserScopeOp::RemoveMember(RemoveMemberUserScopeOpBody {
				kind: _,
				header: _,
				member,
				scope_members_root,
			}) => {
				let state = scope_state().await?;

				if state.owner == *signer && member.is_none() {
					return Err(Error::OwnerAsMember);
				}

				let scope_members = MerkleBPlusTree::<ScopeMembersTree, _>::from_root_hash(
					&state.scope_members_root,
					todo!(),
					Default::default(),
				)
				.await?;

				if scope_members
					.delete(member.as_ref().unwrap_or(signer))
					.await?
					.is_none()
				{
					return Err(Error::Unauthorized);
				};

				if scope_members.root_hash() != scope_members_root.0 {
					return Err(Error::ComputedRootDifferent);
				}
			}
			UserScopeOp::TransferOwnership(_) => {}
			UserScopeOp::PublishVersion(_) => unreachable!(),
			UserScopeOp::SetYanked(SetYankedUserScopeOpBody {
				kind: _,
				header: _,
				pkg,
				version,
				yanked,
				versions_root,
			}) => {
				let state = scope_state().await?;
				let versions = MerkleBPlusTree::<PackageVersionsTree, _>::from_root_hash(
					&state.versions_root,
					todo!(),
					Default::default(),
				)
				.await?;

				let key = (pkg.clone(), version.clone());

				let Some(mut version) = versions.get(&key).await? else {
					return Err(Error::VersionNotFound);
				};

				if version.yank_state == Some(VersionYankState::AdminYanked) {
					return Err(Error::AdminYanked);
				}

				let new_state = if *yanked {
					Some(VersionYankState::Yanked)
				} else {
					None
				};

				if version.yank_state == new_state {
					return Err(Error::AlreadyInState);
				}
				version.yank_state = new_state;

				versions.insert(key, version).await?;

				if versions.root_hash() != versions_root.0 {
					return Err(Error::ComputedRootDifferent);
				}
			}
			UserScopeOp::SetDeprecation(SetDeprecationUserScopeOpBody {
				kind: _,
				header: _,
				pkg,
				reason_hash,
				deprecations_root,
			}) => {
				// TODO: insert plaintext reason, compare hash

				let state = scope_state().await?;
				let deprecations = MerkleBPlusTree::<PackageDeprecationsTree, _>::from_root_hash(
					&state.deprecations_root,
					todo!(),
					Default::default(),
				)
				.await?;

				match reason_hash {
					Some(new_state) => {
						if deprecations
							.insert(pkg.clone(), new_state.clone())
							.await?
							.is_some_and(|o| o == *new_state)
						{
							return Err(Error::AlreadyInState);
						}
					}
					None => {
						if deprecations.delete(pkg).await?.is_none() {
							return Err(Error::AlreadyInState);
						}
					}
				}

				if deprecations.root_hash() != deprecations_root.0 {
					return Err(Error::ComputedRootDifferent);
				}
			}
		};

		let mmr = MMRIVER::<CurrentMerkleHasher, _>::new(scope_size.get(), tx);

		Ok(todo!())
	})
	.await
}
