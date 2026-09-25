use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::post;
use actix_web::web;
use merkle_bplustree::InsertNewError;
use merkleberg::MMRIVER;
use pesde::hash::Hash;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::db::ExistingScopeLockResult;
use pesde_registry_core::db::PermissionWidth;

use crate::AppState;
use crate::api::scope::error::Error;
use crate::api::scope::run_tree;
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
				run_tree(tx, &state, scope_members_root, async |tree| {
					match tree
						.insert_new(member.consenter().clone(), grant.clone())
						.await
					{
						Ok(_) => Ok(()),
						Err(InsertNewError::AlreadyExists) => Err(Error::KeyAlreadyExists),
						Err(InsertNewError::StorageError(e)) => Err(e.into()),
					}
				})
				.await?;
			}
			UserScopeOp::UpdateMemberGrant(UpdateMemberGrantUserScopeOpBody {
				kind: _,
				header: _,
				member,
				grant,
				scope_members_root,
			}) => {
				let state = scope_state().await?;
				run_tree(tx, &state, scope_members_root, async |tree| {
					match tree.insert(member.clone(), grant.clone()).await? {
						Some(g) if g == *grant => Err(Error::GrantNoChange),
						Some(_) => Ok(()),
						None => Err(Error::KeyNotFound),
					}
				})
				.await?;
			}
			UserScopeOp::RotateKey(RotateKeyUserScopeOpBody {
				kind: _,
				header: _,
				new_key,
				scope_members_root,
			}) => {
				let state = scope_state().await?;
				run_tree(tx, &state, scope_members_root, async |tree| {
					let Some(grant) = tree.delete(signer).await? else {
						return Err(Error::Unauthorized);
					};

					match tree.insert_new(new_key.consenter().clone(), grant).await {
						Ok(_) => Ok(()),
						Err(InsertNewError::AlreadyExists) => Err(Error::KeyAlreadyExists),
						Err(InsertNewError::StorageError(e)) => Err(e.into()),
					}
				})
				.await?;
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

				run_tree(tx, &state, scope_members_root, async |tree| {
					tree.delete(member.as_ref().unwrap_or(signer))
						.await?
						.is_some()
						.ok_or(Error::AlreadyInState)
				})
				.await?;
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
				run_tree(tx, &state, versions_root, async |tree| {
					let key = (pkg.clone(), version.clone());

					let Some(mut version) = tree.get(&key).await? else {
						return Err(Error::VersionNotFound);
					};

					if version.yank_state == Some(VersionYankState::AdminYanked) {
						return Err(Error::VersionAdminYanked);
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

					tree.insert(key, version).await?;

					Ok(())
				})
				.await?;
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
				run_tree(
					tx,
					&state,
					deprecations_root,
					async |tree| match reason_hash {
						Some(new_state) => tree
							.insert(pkg.clone(), new_state.clone())
							.await?
							.is_none_or(|o| o != *new_state)
							.ok_or(Error::AlreadyInState),
						None => tree
							.delete(pkg)
							.await?
							.is_some()
							.ok_or(Error::AlreadyInState),
					},
				)
				.await?;
			}
		};

		let mmr = MMRIVER::<CurrentMerkleHasher, _>::new(scope_size.get(), tx);

		Ok(todo!())
	})
	.await
}
