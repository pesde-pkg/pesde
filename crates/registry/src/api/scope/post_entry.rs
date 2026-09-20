use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::post;
use actix_web::web;
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
	let payload = payload.into_inner();

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
			if member.inner().body.consenter == *signer {
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
			if new_key.inner().body.consenter == *signer {
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
			if *signer == new_owner.inner().body.consenter {
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
			}) => {}
			UserScopeOp::UpdateMemberGrant(UpdateMemberGrantUserScopeOpBody {
				kind: _,
				header: _,
				member,
				grant,
				scope_members_root,
			}) => {}
			UserScopeOp::RotateKey(RotateKeyUserScopeOpBody {
				kind: _,
				header: _,
				new_key,
				scope_members_root,
			}) => {}
			UserScopeOp::RemoveMember(RemoveMemberUserScopeOpBody {
				kind: _,
				header: _,
				member,
				scope_members_root,
			}) => {
				if scope_state().await?.owner == *signer && member.is_none() {
					return Err(Error::OwnerAsMember);
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
			}) => {}
			UserScopeOp::SetDeprecation(SetDeprecationUserScopeOpBody {
				kind: _,
				header: _,
				pkg,
				reason_hash,
				deprecations_root,
			}) => {}
		};

		let mmr = MMRIVER::<CurrentMerkleHasher, _>::new(scope_size.get(), tx);

		Ok(todo!())
	})
	.await
}
