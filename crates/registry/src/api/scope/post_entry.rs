use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::post;
use actix_web::web;
use pesde_core::hash::Hash;
use pesde_core::merkle_bplustree::InsertNewError;
use pesde_core::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::db::ExistingScopeLockResult;
use pesde_registry_core::db::PermissionWidth;
use pesde_registry_core::features::scope::Scoped;

use crate::Db;
use crate::api::scope::error::Error;
use crate::api::scope::run_tree;
use crate::shared::db::append_leaf;
use crate::shared::db::run_tx;

#[post("/scope/log/entry")]
pub(super) async fn http_v2(
	db: Db,
	body: web::Json<Signed<UserScopeOp>>,
) -> Result<impl Responder, Error> {
	let entry = handler(db.as_ref(), body.into_inner()).await?;
	Ok(HttpResponse::Ok().json(entry))
}

async fn handler(db: &dyn Backend, payload: Signed<UserScopeOp>) -> Result<ScopeEntry, Error> {
	let UnvalidatedSigned { sig: _, body: op } = payload.inner();
	let UserScopeOpHeader {
		common: ScopeOpHeader {
			scope_id,
			prev_hash: _,
		},
		signer,
	} = op.header();

	let permissions = match op {
		UserScopeOp::AddMember(op) => {
			// only the owner can append this operation, so this simple check avoids a database lookup for completely
			// nonsensical data
			if op.member().consenter() == signer {
				return Err(Error::OwnerAsMember);
			}

			PermissionWidth::Owner
		}
		UserScopeOp::UpdateMemberGrant(op) => {
			if op.member.0 == *signer {
				return Err(Error::OwnerAsMember);
			}

			PermissionWidth::Owner
		}
		UserScopeOp::RotateKey(op) => {
			if op.new_key().consenter() == signer {
				return Err(Error::KeyChangeNoChange);
			}

			PermissionWidth::OnlySelf
		}
		UserScopeOp::RemoveMember(op) => match &op.member {
			Some(k) => {
				if k.0 == *signer {
					return Err(Error::OwnerAsMember);
				}

				PermissionWidth::Owner
			}
			None => PermissionWidth::OnlySelf,
		},
		UserScopeOp::TransferOwnership(op) => {
			if signer == op.new_owner().consenter() {
				return Err(Error::KeyChangeNoChange);
			}

			PermissionWidth::Owner
		}
		UserScopeOp::PublishVersion(_) => {
			return Err(Error::PublishVersionInPostEntry);
		}
		UserScopeOp::SetYanked(op) => PermissionWidth::Package(&op.package_version.0),
		UserScopeOp::SetDeprecation(op) => PermissionWidth::Package(&op.pkg),
	};

	if let Some(grant) = grant(op)
		&& grant.0.iter().any(|e| e.plaintext().is_none())
	{
		return Err(Error::GrantPlaintextRequired);
	}

	let (tx, scope_size) = match db
		.begin_write_existing(scope_id, signer, permissions)
		.await?
	{
		ExistingScopeLockResult::Ok { tx, scope_size } => (tx, scope_size),
		ExistingScopeLockResult::DoesntExist => return Err(Error::ScopeNotFound),
		ExistingScopeLockResult::Unauthorized => return Err(Error::Unauthorized),
	};

	run_tx(tx, async move |tx| {
		let UnvalidatedSigned { sig: _, body: op } = payload.inner();
		let UserScopeOpHeader {
			common: ScopeOpHeader {
				scope_id,
				prev_hash: given_prev_hash,
			},
			signer,
		} = op.header();

		let prev_entry = tx
			.scope_log_entry(scope_id.clone(), scope_size.get())
			.await?
			.ok_or(Error::ScopeNotFound)?;

		let prev_entry_hash =
			Hash::digest(given_prev_hash.algorithm(), prev_entry.canonical_bytes());
		if *given_prev_hash != prev_entry_hash {
			return Err(Error::InvalidPrevHash);
		}

		let scope_state = async || tx.scope_state(scope_id).await?.ok_or(Error::ScopeNotFound);

		match op {
			UserScopeOp::AddMember(
				op @ AddMemberUserScopeOpBody {
					grant,
					scope_members_root,
					..
				},
			) => {
				let state = scope_state().await?;
				run_tree(
					scope_id,
					tx,
					&state,
					scope_members_root,
					async |tree| match tree
						.insert_new(
							ScopeMember(op.member().consenter().clone()),
							ScopeMemberValue {
								grant: grant.clone(),
							},
						)
						.await
					{
						Ok(_) => Ok(()),
						Err(InsertNewError::AlreadyExists) => Err(Error::KeyAlreadyExists),
						Err(InsertNewError::StorageError(e)) => Err(e.into()),
					},
				)
				.await?;
			}
			UserScopeOp::UpdateMemberGrant(UpdateMemberGrantUserScopeOpBody {
				header: _,
				member,
				grant,
				scope_members_root,
			}) => {
				let state = scope_state().await?;
				run_tree(
					scope_id,
					tx,
					&state,
					scope_members_root,
					async |tree| match tree
						.insert(
							member.clone(),
							ScopeMemberValue {
								grant: grant.clone(),
							},
						)
						.await?
					{
						Some(v) if v.grant == *grant => Err(Error::GrantNoChange),
						Some(_) => Ok(()),
						None => Err(Error::KeyNotFound),
					},
				)
				.await?;
			}
			UserScopeOp::RotateKey(
				op @ RotateKeyUserScopeOpBody {
					scope_members_root, ..
				},
			) => {
				let state = scope_state().await?;
				run_tree(scope_id, tx, &state, scope_members_root, async |tree| {
					let Some(value) = tree.delete(&ScopeMember(op.signer().clone())).await? else {
						return Err(Error::Unauthorized);
					};

					match tree
						.insert_new(ScopeMember(op.new_key().consenter().clone()), value)
						.await
					{
						Ok(_) => Ok(()),
						Err(InsertNewError::AlreadyExists) => Err(Error::KeyAlreadyExists),
						Err(InsertNewError::StorageError(e)) => Err(e.into()),
					}
				})
				.await?;
			}
			UserScopeOp::RemoveMember(RemoveMemberUserScopeOpBody {
				header: _,
				member,
				scope_members_root,
			}) => {
				let state = scope_state().await?;
				if state.owner == *signer && member.is_none() {
					return Err(Error::OwnerAsMember);
				}

				run_tree(scope_id, tx, &state, scope_members_root, async |tree| {
					tree.delete(member.as_ref().unwrap_or(&ScopeMember(signer.clone())))
						.await?
						.is_some()
						.ok_or(Error::AlreadyInState)
				})
				.await?;
			}
			UserScopeOp::TransferOwnership(_) => {}
			UserScopeOp::PublishVersion(_) => unreachable!(),
			UserScopeOp::SetYanked(SetYankedUserScopeOpBody {
				header: _,
				package_version,
				yanked,
				versions_root,
			}) => {
				let state = scope_state().await?;
				run_tree(scope_id, tx, &state, versions_root, async |tree| {
					let Some(mut version) = tree.get(package_version).await? else {
						return Err(Error::VersionNotFound);
					};

					if version.yank_state == Some(VersionYankState::AdminYanked) {
						return Err(Error::VersionAdminYanked);
					}

					let new_state = (*yanked).then_some(VersionYankState::Yanked);

					if version.yank_state == new_state {
						return Err(Error::AlreadyInState);
					}
					version.yank_state = new_state;

					tree.insert(package_version.clone(), version).await?;

					Ok(())
				})
				.await?;
			}
			UserScopeOp::SetDeprecation(SetDeprecationUserScopeOpBody {
				header: _,
				pkg,
				state: value,
				deprecations_root,
			}) => {
				let state = scope_state().await?;
				run_tree(
					scope_id,
					tx,
					&state,
					deprecations_root,
					async |tree| match value {
						Some(new_state) => tree
							.insert(PackageDeprecation(pkg.clone()), new_state.clone())
							.await?
							.is_none_or(|o| o != *new_state)
							.ok_or(Error::AlreadyInState),
						None => tree
							.delete(&PackageDeprecation(pkg.clone()))
							.await?
							.is_some()
							.ok_or(Error::AlreadyInState),
					},
				)
				.await?;
			}
		}

		let entry = Entry {
			pos: scope_size.get(),
			published_at: Default::default(),
			payload: ScopeEntryPayload::User(payload),
		};
		let scope_id = match &entry.payload {
			ScopeEntryPayload::User(e) => &e.inner().body.header().scope_id,
			ScopeEntryPayload::Admin(e) => &e.header().scope_id,
		};

		let new_size = append_leaf(Scoped(scope_id, &mut *tx), scope_size.get(), &entry).await?;
		tx.set_scope_log_size(scope_id, new_size).await?;
		tx.insert_scope_entry(&entry).await?;

		Ok(entry)
	})
	.await
}

fn grant(op: &UserScopeOp) -> Option<&ScopeGrant> {
	match op {
		UserScopeOp::AddMember(AddMemberUserScopeOpBody { grant, .. })
		| UserScopeOp::UpdateMemberGrant(UpdateMemberGrantUserScopeOpBody { grant, .. }) => Some(grant),
		_ => None,
	}
}
