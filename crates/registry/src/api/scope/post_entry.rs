use actix_web::HttpResponse;
use actix_web::Responder;
use actix_web::post;
use actix_web::web;
use merkleberg::MMRIVER;
use pesde::source::pesde::registry::*;
use pesde_registry_core::db::Backend;
use pesde_registry_core::db::ExistingScopeLockResult;
use pesde_registry_core::db::PermissionWidth;

use crate::AppState;
use crate::api::scope::error::Error;

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

	let UnvalidatedSigned { sig, body: op } = &payload;

	let permissions = match op {
		UserScopeOp::AddMember(_) => PermissionWidth::Owner,
		UserScopeOp::UpdateMemberGrant(_) => PermissionWidth::Owner,
		UserScopeOp::RotateKey(_) => PermissionWidth::OnlySelf,
		UserScopeOp::RemoveMember(_) => PermissionWidth::Owner,
		UserScopeOp::TransferOwnership(_) => PermissionWidth::Owner,
		UserScopeOp::PublishVersion(_) => {
			return Err(Error::PublishVersionInPostEntry);
		}
		UserScopeOp::SetYanked(op) => PermissionWidth::Package(&op.pkg),
		UserScopeOp::SetDeprecation(op) => PermissionWidth::Package(&op.pkg),
	};

	let (tx, scope_size) = match db
		.begin_write_existing(&op.header().scope_id, &op.header().signer, permissions)
		.await?
	{
		ExistingScopeLockResult::Ok { tx, scope_size } => (tx, scope_size),
		ExistingScopeLockResult::DoesntExist => return Err(Error::ScopeNotFound),
		ExistingScopeLockResult::Unauthorized => return Err(Error::Unauthorized),
	};

	match op {
		UserScopeOp::AddMember(AddMemberUserScopeOpBody {
			member,
			grant,
			consent,
			nonce,
			scope_members_root,
		}) => {}
		UserScopeOp::UpdateMemberGrant(UpdateMemberGrantUserScopeOpBody {
			member,
			grant,
			scope_members_root,
		}) => {}
		UserScopeOp::RotateKey(RotateKeyUserScopeOpBody {
			new_key,
			new_key_proof,
			nonce,
			scope_members_root,
		}) => {}
		UserScopeOp::RemoveMember(RemoveMemberUserScopeOpBody {
			member,
			scope_members_root,
		}) => {}
		UserScopeOp::TransferOwnership(TransferOwnershipUserScopeOpBody {
			new_owner,
			new_owner_consent,
			nonce,
		}) => {}
		UserScopeOp::PublishVersion(_) => unreachScopeable!(),
		UserScopeOp::SetYanked(SetYankedUserScopeOpBody {
			pkg,
			version,
			yanked,
			versions_root,
		}) => {}
		UserScopeOp::SetDeprecation(SetDeprecationUserScopeOpBody {
			pkg,
			reason_hash,
			deprecations_root,
		}) => {}
	};

	let mmr = MMRIVER::<CurrentMerkleHasher, _>::new(scope_size, tx);

	Ok(todo!())
}
