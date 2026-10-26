use std::collections::BTreeSet;
use std::num::NonZero;
use std::sync::Arc;

use anyhow::Context as _;
use arrayvec::ArrayVec;
use async_trait::async_trait;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use merkle_bplustree::HasherOutput;
use merkle_bplustree::TreeConfig;
use merkle_bplustree::node::TreeInternalNode;
use merkle_bplustree::node::TreeLeafNode;
use merkle_bplustree::node::TreeNode;
use pesde::bounded::Bounded;
use pesde::bounded::BoundedBTreeSet;
use pesde::hash::Blake3Hash;
use pesde::hash::Hash;
use pesde::signature::KeyKind;
use pesde::signature::PublicKey;
use pesde::signature::Signature;
use pesde::signature::SignatureKind;
use pesde::source::pesde::registry::*;
use pesde_registry_core::features::scope::ScopeReadRepository;
use pesde_registry_core::features::scope::ScopeTreeReadRepository;
use pesde_registry_core::features::scope::ScopeTreeWriteRepository;
use pesde_registry_core::features::scope::ScopeWriteRepository;
use sqlx::QueryBuilder;
use sqlx::prelude::Type;
use sqlx::query;
use sqlx::types::Text;
use uuid::Uuid;

use crate::AsExecutor;
use crate::MySqlBackend;
use crate::MySqlTransaction;
use crate::util::chrono_to_jiff;
use crate::util::insert_chunked;
use crate::util::jiff_to_chrono;

#[derive(Debug, Clone, Copy, Type)]
#[sqlx(rename_all = "snake_case")]
enum ScopeNamespace {
	Signed,
	Admin,
}

#[derive(Debug, Clone, Copy, Type)]
#[sqlx(rename_all = "snake_case")]
enum SignedOpKind {
	AddMember,
	UpdateMemberGrant,
	RotateKey,
	RemoveMember,
	TransferOwnership,
	PublishVersion,
	SetYanked,
	SetDeprecation,
}

#[derive(Debug, Clone, Copy, Type)]
#[sqlx(rename_all = "snake_case")]
enum AdminOpKind {
	TransferOwnership,
	SetYanked,
}

#[derive(Debug, Clone, Copy, Type)]
#[sqlx(rename_all = "snake_case")]
enum TreeKind {
	Internal,
	Leaf,
}

async fn scope_log_size(
	mut tx: impl AsExecutor,
	scope_id: &ScopeId,
) -> anyhow::Result<Option<NonZero<u64>>> {
	let record = query!(
		r"
SELECT scope.log_size AS `log_size: NonZero<u64>`
FROM scope
INNER JOIN global_log_scope_genesis_entry ON global_log_scope_genesis_entry.pos=scope.genesis_pos
WHERE global_log_scope_genesis_entry.name_blake3 = ?",
		scope_id.hash().0.as_slice()
	)
	.fetch_optional(tx.as_executor())
	.await?;

	Ok(record.map(|r| r.log_size))
}

async fn scope_grant(tx: &mut impl AsExecutor, grant_blake3: &[u8]) -> anyhow::Result<ScopeGrant> {
	let grant = query!(
		r"
SELECT member_grant_package.local_name_blake3, plaintext_name.plaintext
FROM member_grant_package
INNER JOIN plaintext_name ON plaintext_name.blake3=member_grant_package.local_name_blake3
WHERE member_grant_package.grant_blake3 = ?",
		grant_blake3,
	)
	.fetch(tx.as_executor())
	.map(|result| match result {
		Ok(r) => {
			let id = CurrentHash::new(r.local_name_blake3).context("corrupt local name id")?;

			ScopeGrantEntry::new(
				LocalNameId::new(id),
				(!r.plaintext.is_empty())
					.then(|| r.plaintext.parse())
					.transpose()?,
			)
			.map_err(anyhow::Error::from)
		}
		Err(e) => Err(e.into()),
	})
	.try_collect::<BTreeSet<_>>()
	.await?;

	Ok(ScopeGrant(BoundedBTreeSet::new(grant)?))
}

async fn scope_log_entry(
	mut tx: impl AsExecutor,
	scope_id: ScopeId,
	pos: u64,
) -> anyhow::Result<Option<ScopeEntry>> {
	let result = query!(
		r"
SELECT scope_log_entry.scope AS scope_pos, ns AS `ns: ScopeNamespace`, published_at, prev_hash, scope_members_root_blake3, versions_root_blake3, deprecations_root_blake3
FROM scope_log_entry
INNER JOIN global_log_scope_genesis_entry ON global_log_scope_genesis_entry.pos=scope_log_entry.scope
WHERE global_log_scope_genesis_entry.name_blake3 = ? AND scope_log_entry.pos = ?",
		scope_id.hash().0.as_slice(),
		pos,
	)
	.fetch_optional(tx.as_executor())
	.await?;

	let Some(record) = result else {
		return Ok(None);
	};

	let header = ScopeOpHeader {
		scope_id,
		prev_hash: record.prev_hash.parse()?,
	};

	let scope_members_root = Blake3Hash::new(record.scope_members_root_blake3)
		.context("corrupt scope_members_root")
		.map(ScopeMembersTree)?;
	let package_versions_root = Blake3Hash::new(record.versions_root_blake3)
		.context("corrupt package_versions_root")
		.map(PackageVersionsTree)?;
	let package_deprecations_root = Blake3Hash::new(record.deprecations_root_blake3)
		.context("corrupt package_deprecations_root")
		.map(PackageDeprecationsTree)?;

	let payload = match record.ns {
		ScopeNamespace::Signed => ScopeEntryPayload::User({
			let signed_record = query!(
				r"
SELECT 
	signed_scope_log_entry.op_kind AS `op_kind: SignedOpKind`,
	signed_scope_log_entry.sig_algorithm AS `sig_algorithm: Text<SignatureKind>`,
	signed_scope_log_entry.sig,
	public_key.algorithm AS `kind: Text<KeyKind>`,
	public_key.data
FROM signed_scope_log_entry
INNER JOIN public_key ON public_key.id=signed_scope_log_entry.signer
WHERE signed_scope_log_entry.scope = ? AND signed_scope_log_entry.pos = ?",
				record.scope_pos,
				pos,
			)
			.fetch_one(tx.as_executor())
			.await?;

			let header = UserScopeOpHeader {
				common: header,
				signer: PublicKey::new(signed_record.kind.0, signed_record.data)
					.context("corrupt signer")?,
			};

			let body = match signed_record.op_kind {
				SignedOpKind::AddMember => {
					let r = query!(
						r"
SELECT
	signed_add_member_scope_log_entry.consent_algorithm AS `consent_algorithm: Text<SignatureKind>`,
	signed_add_member_scope_log_entry.consent,
	signed_add_member_scope_log_entry.nonce AS `nonce: Uuid`,
	signed_add_member_scope_log_entry.grant_blake3,
	public_key.algorithm AS `kind: Text<KeyKind>`,
	public_key.data
FROM signed_add_member_scope_log_entry
INNER JOIN public_key ON public_key.id=signed_add_member_scope_log_entry.member
WHERE signed_add_member_scope_log_entry.scope = ? AND signed_add_member_scope_log_entry.pos = ?",
						record.scope_pos,
						pos,
					)
					.fetch_one(tx.as_executor())
					.await?;

					let member = UnvalidatedSigned {
						sig: Signature::new(r.consent_algorithm.0, r.consent)
							.context("corrupt signature")?,
						body: UnvalidatedConsent {
							nonce: r.nonce,
							consenter: PublicKey::new(r.kind.0, r.data)
								.context("corrupt scope member")?,
						},
					};

					let grant = scope_grant(&mut tx, &r.grant_blake3).await?;

					UserScopeOp::AddMember(AddMemberUserScopeOpBody::new(
						header,
						member,
						grant,
						scope_members_root,
					)?)
				}
				SignedOpKind::UpdateMemberGrant => {
					let r = query!(
						r"
SELECT
	signed_update_member_grant_scope_log_entry.grant_blake3,
	public_key.algorithm AS `kind: Text<KeyKind>`,
	public_key.data
FROM signed_update_member_grant_scope_log_entry
INNER JOIN public_key ON public_key.id=signed_update_member_grant_scope_log_entry.member
WHERE signed_update_member_grant_scope_log_entry.scope = ? AND signed_update_member_grant_scope_log_entry.pos = ?",
						record.scope_pos,
						pos,
					)
					.fetch_one(tx.as_executor())
					.await?;

					UserScopeOp::UpdateMemberGrant(UpdateMemberGrantUserScopeOpBody {
						header,
						member: ScopeMember(
							PublicKey::new(r.kind.0, r.data).context("corrupt scope member")?,
						),
						grant: scope_grant(&mut tx, &r.grant_blake3).await?,
						scope_members_root,
					})
				}
				SignedOpKind::RotateKey => {
					let r = query!(
						r"
SELECT
	signed_rotate_key_scope_log_entry.consent_algorithm AS `consent_algorithm: Text<SignatureKind>`,
	signed_rotate_key_scope_log_entry.consent,
	signed_rotate_key_scope_log_entry.nonce AS `nonce: Uuid`,
	public_key.algorithm AS `kind: Text<KeyKind>`,
	public_key.data
FROM signed_rotate_key_scope_log_entry
INNER JOIN public_key ON public_key.id=signed_rotate_key_scope_log_entry.new_key
WHERE signed_rotate_key_scope_log_entry.scope = ? AND signed_rotate_key_scope_log_entry.pos = ?",
						record.scope_pos,
						pos,
					)
					.fetch_one(tx.as_executor())
					.await?;

					let new_key = UnvalidatedSigned {
						sig: Signature::new(r.consent_algorithm.0, r.consent)
							.context("corrupt signature")?,
						body: UnvalidatedConsent {
							nonce: r.nonce,
							consenter: PublicKey::new(r.kind.0, r.data)
								.context("corrupt new key")?,
						},
					};

					UserScopeOp::RotateKey(RotateKeyUserScopeOpBody::new(
						header,
						new_key,
						scope_members_root,
					)?)
				}
				SignedOpKind::RemoveMember => {
					let r = query!(
						r"
SELECT
	public_key.algorithm AS `kind: Text<KeyKind>`,
	public_key.data AS `data`
FROM signed_remove_member_scope_log_entry
LEFT JOIN public_key ON public_key.id=signed_remove_member_scope_log_entry.member
WHERE signed_remove_member_scope_log_entry.scope = ? AND signed_remove_member_scope_log_entry.pos = ?",
						record.scope_pos,
						pos,
					)
					.fetch_one(tx.as_executor())
					.await?;

					let member = r
						.kind
						.zip(r.data)
						.map(|(kind, data)| {
							PublicKey::new(kind.0, data)
								.context("corrupt removed member")
								.map(ScopeMember)
						})
						.transpose()?;

					UserScopeOp::RemoveMember(RemoveMemberUserScopeOpBody {
						header,
						member,
						scope_members_root,
					})
				}
				SignedOpKind::TransferOwnership => {
					let r = query!(
						r"
SELECT
	signed_transfer_ownership_scope_log_entry.consent_algorithm AS `consent_algorithm: Text<SignatureKind>`,
	signed_transfer_ownership_scope_log_entry.consent,
	signed_transfer_ownership_scope_log_entry.nonce AS `nonce: Uuid`,
	public_key.algorithm AS `kind: Text<KeyKind>`,
	public_key.data
FROM signed_transfer_ownership_scope_log_entry
INNER JOIN scope_log_entry ON scope_log_entry.scope=signed_transfer_ownership_scope_log_entry.scope AND scope_log_entry.pos=signed_transfer_ownership_scope_log_entry.pos
INNER JOIN public_key ON public_key.id=scope_log_entry.owner
WHERE signed_transfer_ownership_scope_log_entry.scope = ? AND signed_transfer_ownership_scope_log_entry.pos = ?",
						record.scope_pos,
						pos,
					)
					.fetch_one(tx.as_executor())
					.await?;

					let new_owner = UnvalidatedSigned {
						sig: Signature::new(r.consent_algorithm.0, r.consent)
							.context("corrupt signature")?,
						body: UnvalidatedConsent {
							nonce: r.nonce,
							consenter: PublicKey::new(r.kind.0, r.data)
								.context("corrupt new owner")?,
						},
					};

					UserScopeOp::TransferOwnership(TransferOwnershipUserScopeOpBody::new(
						header, new_owner,
					)?)
				}
				SignedOpKind::PublishVersion => {
					let r = query!(
						r"
SELECT package.local_name_blake3, package_version.version, package_version.archive_hash
FROM signed_publish_version_scope_log_entry
INNER JOIN package_version ON package_version.scope=signed_publish_version_scope_log_entry.scope AND package_version.pos=signed_publish_version_scope_log_entry.pos
INNER JOIN package ON package.scope=package_version.scope AND package.pos=package_version.package_pos
WHERE signed_publish_version_scope_log_entry.scope = ? AND signed_publish_version_scope_log_entry.pos = ?",
						record.scope_pos,
						pos,
					)
					.fetch_one(tx.as_executor())
					.await?;

					UserScopeOp::PublishVersion(PublishVersionUserScopeOpBody {
						header,
						package_version: PackageVersion(
							LocalNameId::new(
								CurrentHash::new(r.local_name_blake3)
									.context("illegal local name id")?,
							),
							r.version.parse()?,
						),
						archive_hash: r.archive_hash.parse()?,
						versions_root: package_versions_root,
					})
				}
				SignedOpKind::SetYanked => {
					let r = query!(
						r"
SELECT package.local_name_blake3, package_version.version, signed_set_yanked_scope_log_entry.yanked AS `yanked: bool`
FROM signed_set_yanked_scope_log_entry
INNER JOIN package_version ON package_version.scope=signed_set_yanked_scope_log_entry.scope AND package_version.pos=signed_set_yanked_scope_log_entry.version_pos
INNER JOIN package ON package.scope=package_version.scope AND package.pos=package_version.package_pos
WHERE signed_set_yanked_scope_log_entry.scope = ? AND signed_set_yanked_scope_log_entry.pos = ?",
						record.scope_pos,
						pos,
					)
					.fetch_one(tx.as_executor())
					.await?;

					UserScopeOp::SetYanked(SetYankedUserScopeOpBody {
						header,
						package_version: PackageVersion(
							LocalNameId::new(
								CurrentHash::new(r.local_name_blake3)
									.context("illegal local name id")?,
							),
							r.version.parse()?,
						),
						yanked: r.yanked,
						versions_root: package_versions_root,
					})
				}
				SignedOpKind::SetDeprecation => {
					let r = query!(
						r"
SELECT package.local_name_blake3, deprecation_reason.hash, deprecation_reason.plaintext AS `plaintext: Box<str>`
FROM signed_set_deprecation_scope_log_entry
INNER JOIN package ON package.scope=signed_set_deprecation_scope_log_entry.scope AND package.pos=signed_set_deprecation_scope_log_entry.package_pos
LEFT JOIN deprecation_reason ON deprecation_reason.id=signed_set_deprecation_scope_log_entry.reason_id
WHERE signed_set_deprecation_scope_log_entry.scope = ? AND signed_set_deprecation_scope_log_entry.pos = ?",
						record.scope_pos,
						pos,
					)
					.fetch_one(tx.as_executor())
					.await?;

					let state = r
						.hash
						.zip(r.plaintext)
						.map(|(hash, plaintext)| {
							anyhow::Ok(PackageDeprecationValue {
								reason: PackageDeprecationReason::new(
									hash.parse()?,
									Bounded::new(plaintext)?,
								)?,
							})
						})
						.transpose()?;

					UserScopeOp::SetDeprecation(SetDeprecationUserScopeOpBody {
						header,
						pkg: LocalNameId::new(
							CurrentHash::new(r.local_name_blake3)
								.context("illegal local name id")?,
						),
						state,
						deprecations_root: package_deprecations_root,
					})
				}
			};

			Signed::new(UnvalidatedSigned {
				sig: Signature::new(signed_record.sig_algorithm.0, signed_record.sig)
					.context("corrupt signature")?,
				body,
			})?
		}),
		ScopeNamespace::Admin => ScopeEntryPayload::Admin({
			let admin_record = query!(
				r"
SELECT op_kind AS `op_kind: AdminOpKind`
FROM admin_scope_log_entry
WHERE scope = ? AND pos = ?",
				record.scope_pos,
				pos,
			)
			.fetch_one(tx.as_executor())
			.await?;

			match admin_record.op_kind {
				AdminOpKind::TransferOwnership => {
					let r = query!(
						r"
SELECT public_key.algorithm AS `kind: Text<KeyKind>`, public_key.data
FROM admin_transfer_ownership_scope_log_entry
INNER JOIN public_key ON public_key.id=admin_transfer_ownership_scope_log_entry.new_owner
WHERE admin_transfer_ownership_scope_log_entry.scope = ? AND admin_transfer_ownership_scope_log_entry.pos = ?",
						record.scope_pos,
						pos,
					)
					.fetch_one(tx.as_executor())
					.await?;

					AdminScopeOp::TransferOwnership(TransferOwnershipAdminScopeOpBody {
						header,
						new_owner: PublicKey::new(r.kind.0, r.data).context("corrupt new owner")?,
					})
				}
				AdminOpKind::SetYanked => {
					let r = query!(
						r"
SELECT package.local_name_blake3, package_version.version, admin_set_yanked_scope_log_entry.yanked AS `yanked: bool`
FROM admin_set_yanked_scope_log_entry
INNER JOIN package_version ON package_version.scope=admin_set_yanked_scope_log_entry.scope AND package_version.pos=admin_set_yanked_scope_log_entry.version_pos
INNER JOIN package ON package.scope=package_version.scope AND package.pos=package_version.package_pos
WHERE admin_set_yanked_scope_log_entry.scope = ? AND admin_set_yanked_scope_log_entry.pos = ?",
						record.scope_pos,
						pos,
					)
					.fetch_one(tx.as_executor())
					.await?;

					AdminScopeOp::SetYanked(SetYankedAdminScopeOpBody {
						header,
						pkg: LocalNameId::new(
							CurrentHash::new(r.local_name_blake3)
								.context("illegal local name id")?,
						),
						version: r.version.parse()?,
						yanked: r.yanked,
						versions_root: package_versions_root,
					})
				}
			}
		}),
	};

	Ok(Some(Entry {
		pos,
		published_at: chrono_to_jiff(record.published_at)?,
		payload,
	}))
}

async fn scope_state(
	mut tx: impl AsExecutor,
	scope_id: &ScopeId,
) -> anyhow::Result<Option<ScopeStateResponse>> {
	let result = query!(
		r"
SELECT
	scope_log_entry.scope_members_root_blake3,
	scope_log_entry.versions_root_blake3,
	scope_log_entry.deprecations_root_blake3,
	public_key.algorithm AS `kind: Text<KeyKind>`,
	public_key.data
FROM scope_log_entry
INNER JOIN global_log_scope_genesis_entry ON global_log_scope_genesis_entry.pos=scope_log_entry.scope
INNER JOIN public_key ON public_key.id=scope_log_entry.owner
WHERE global_log_scope_genesis_entry.name_blake3 = ?
ORDER BY scope_log_entry.pos DESC
LIMIT 1",
		scope_id.hash().0.as_slice(),
	)
	.fetch_optional(tx.as_executor())
	.await?;

	let Some(record) = result else {
		return Ok(None);
	};

	Ok(Some(ScopeStateResponse {
		owner: PublicKey::new(record.kind.0, record.data).context("corrupt owner key")?,
		scope_members_root: Blake3Hash::new(record.scope_members_root_blake3)
			.context("corrupt scope_members_root")
			.map(ScopeMembersTree)?,
		package_versions_root: Blake3Hash::new(record.versions_root_blake3)
			.context("corrupt package_versions_root")
			.map(PackageVersionsTree)?,
		package_deprecations_root: Blake3Hash::new(record.deprecations_root_blake3)
			.context("corrupt package_deprecations_root")
			.map(PackageDeprecationsTree)?,
	}))
}

async fn scope_log_node(
	mut tx: impl AsExecutor,
	scope_id: &ScopeId,
	pos: u64,
) -> anyhow::Result<Option<CurrentHash>> {
	let result = query!(
		"SELECT scope_log_node.blake3 FROM scope_log_node INNER JOIN global_log_scope_genesis_entry ON global_log_scope_genesis_entry.pos=scope_log_node.scope WHERE global_log_scope_genesis_entry.name_blake3 = ? AND scope_log_node.pos = ?",
		scope_id.hash().0.as_slice(),
		pos,
	)
		.fetch_optional(tx.as_executor())
		.await?;

	let Some(record) = result else {
		return Ok(None);
	};

	Ok(Some(
		Blake3Hash::new(record.blake3).context("corrupt node blake3")?,
	))
}

async fn scope_log_nodes(
	mut tx: impl AsExecutor,
	scope_id: &ScopeId,
	positions: Vec<u64>,
) -> anyhow::Result<Vec<Option<CurrentHash>>> {
	let mut nodes = Vec::new();
	nodes.reserve_exact(positions.len());

	let mut query = QueryBuilder::new(
		"SELECT scope_log_node.blake3 FROM scope_log_node INNER JOIN global_log_scope_genesis_entry ON global_log_scope_genesis_entry.pos=scope_log_node.scope WHERE global_log_scope_genesis_entry.name_blake3 = ",
	);
	query.push_bind(scope_id.hash().0.as_slice());
	query.push(" AND scope_log_node.pos IN (");

	let mut separated = query.separated(',');

	for pos in &positions {
		separated.push_bind(pos);
	}

	query.push(") ORDER BY FIELD(scope_log_node.pos, ");

	let mut separated = query.separated(',');

	for pos in &positions {
		separated.push_bind(pos);
	}

	query.push(')');

	let mut query = query
		.build_query_as::<(Box<[u8]>,)>()
		.fetch(tx.as_executor());
	while let Some((bytes,)) = query.try_next().await? {
		nodes.push(Some(Blake3Hash::new(bytes)?));
	}

	Ok(nodes)
}

async fn scope_members_node(
	mut tx: impl AsExecutor,
	hash: &CurrentHash,
) -> anyhow::Result<Option<TreeNode<ScopeMembersTree>>> {
	let Some(record) = query!(
		"SELECT kind AS `kind: TreeKind` FROM scope_members_node WHERE blake3 = ?",
		hash.0.as_slice()
	)
	.fetch_optional(tx.as_executor())
	.await?
	else {
		return Ok(None);
	};

	let node = match record.kind {
		TreeKind::Internal => {
			let keys = query!(
				r"
SELECT public_key.algorithm AS `algorithm: Text<KeyKind>`, public_key.data
FROM scope_members_internal_node_key AS `key`
INNER JOIN public_key ON public_key.id=key.member
WHERE key.node_blake3 = ?
ORDER BY key.idx",
				hash.0.as_slice()
			)
			.fetch(tx.as_executor())
			.map(|result| match result {
				Ok(r) => PublicKey::new(r.algorithm.0, r.data)
					.context("corrupt public key")
					.map(ScopeMember),
				Err(e) => Err(e.into()),
			})
			.try_collect::<ArrayVec<_, _>>()
			.await?;

			let children = query!(
				r"
SELECT child_blake3
FROM scope_members_internal_node_child
WHERE node_blake3 = ?
ORDER BY idx",
				hash.0.as_slice()
			)
			.fetch(tx.as_executor())
			.map(|result| match result {
				Ok(r) => CurrentHash::new(r.child_blake3).context("corrupt child hash"),
				Err(e) => Err(e.into()),
			})
			.try_collect::<ArrayVec<_, _>>()
			.await?;

			let node =
				TreeInternalNode::new(keys, children).context("tree internal node is illegal")?;
			TreeNode::Internal(Arc::new(node))
		}
		TreeKind::Leaf => {
			let mut keys = ArrayVec::new();
			let mut values = ArrayVec::new();

			let mut query = query!(
				r"
SELECT public_key.algorithm AS `algorithm: Text<KeyKind>`, public_key.data, plaintext_name.blake3, plaintext_name.plaintext
FROM scope_members_leaf_node_entry AS `entry`
INNER JOIN public_key ON public_key.id=entry.member
LEFT JOIN member_grant_package ON member_grant_package.grant_blake3=entry.grant_blake3
LEFT JOIN plaintext_name ON plaintext_name.blake3=member_grant_package.local_name_blake3
WHERE entry.node_blake3 = ?
ORDER BY entry.idx",
				hash.0.as_slice(),
			)
			.fetch(tx.as_executor());

			let mut grant = None;

			while let Some(r) = query.try_next().await? {
				let key = PublicKey::new(r.algorithm.0, r.data)
					.context("corrupt public key")
					.map(ScopeMember)?;

				let grant = if keys.last().is_none_or(|k| *k != key) {
					keys.push(key);
					if let Some(grant) = grant.take() {
						values.push(ScopeMemberValue {
							grant: ScopeGrant(BoundedBTreeSet::new(grant)?),
						});
					}
					grant.insert(Default::default())
				} else {
					grant.as_mut().unwrap()
				};

				if let Some((pkg, plaintext)) = r.blake3.zip(r.plaintext) {
					let pkg =
						LocalNameId::new(CurrentHash::new(pkg).context("illegal local name id")?);
					let plaintext = (!plaintext.is_empty())
						.then(|| plaintext.parse())
						.transpose()?;

					grant.insert(ScopeGrantEntry::new(pkg, plaintext)?);
				}
			}

			if let Some(grant) = grant {
				values.push(ScopeMemberValue {
					grant: ScopeGrant(BoundedBTreeSet::new(grant)?),
				});
			}

			let node = TreeLeafNode::new(keys, values).context("tree leaf node is illegal")?;
			TreeNode::Leaf(Arc::new(node))
		}
	};

	Ok(Some(node))
}

async fn package_versions_node(
	mut tx: impl AsExecutor,
	hash: &CurrentHash,
) -> anyhow::Result<Option<TreeNode<PackageVersionsTree>>> {
	let Some(record) = query!(
		"SELECT kind AS `kind: TreeKind` FROM versions_node WHERE blake3 = ?",
		hash.0.as_slice()
	)
	.fetch_optional(tx.as_executor())
	.await?
	else {
		return Ok(None);
	};

	let node = match record.kind {
		TreeKind::Internal => {
			let keys = query!(
				r"
SELECT package.local_name_blake3, package_version.version
FROM versions_internal_node_key
INNER JOIN package_version ON package_version.scope=versions_internal_node_key.scope AND package_version.pos=versions_internal_node_key.version_pos
INNER JOIN package ON package.scope=package_version.scope AND package.pos=package_version.package_pos
WHERE versions_internal_node_key.node_blake3 = ?
ORDER BY versions_internal_node_key.idx",
				hash.0.as_slice()
			)
			.fetch(tx.as_executor())
			.map(|result| {
				let r = result?;
				Ok::<_, anyhow::Error>(PackageVersion(
					LocalNameId::new(
						CurrentHash::new(r.local_name_blake3).context("illegal local name id")?,
					),
					r.version.parse()?,
				))
			})
			.try_collect::<ArrayVec<_, _>>()
			.await?;

			let children = query!(
				r"
SELECT child_blake3
FROM versions_internal_node_child
WHERE node_blake3 = ?
ORDER BY idx",
				hash.0.as_slice()
			)
			.fetch(tx.as_executor())
			.map(|result| match result {
				Ok(r) => CurrentHash::new(r.child_blake3).context("corrupt child hash"),
				Err(e) => Err(e.into()),
			})
			.try_collect::<ArrayVec<_, _>>()
			.await?;

			let node =
				TreeInternalNode::new(keys, children).context("tree internal node is illegal")?;
			TreeNode::Internal(Arc::new(node))
		}
		TreeKind::Leaf => {
			let mut keys = ArrayVec::new();
			let mut values = ArrayVec::new();

			let mut query = query!(
				r"
SELECT package.local_name_blake3, package_version.version, package_version.archive_hash, versions_leaf_node_entry.yank_state AS `yank_state: VersionYankState`
FROM versions_leaf_node_entry
INNER JOIN package_version ON package_version.scope=versions_leaf_node_entry.scope AND package_version.pos=versions_leaf_node_entry.version_pos
INNER JOIN package ON package.scope=package_version.scope AND package.pos=package_version.package_pos
WHERE versions_leaf_node_entry.node_blake3 = ?
ORDER BY versions_leaf_node_entry.idx",
				hash.0.as_slice()
			)
			.fetch(tx.as_executor());

			while let Some(r) = query.try_next().await? {
				keys.push(PackageVersion(
					LocalNameId::new(
						CurrentHash::new(r.local_name_blake3).context("illegal local name id")?,
					),
					r.version.parse()?,
				));
				values.push(PackageVersionValue {
					archive_hash: r.archive_hash.parse()?,
					yank_state: r.yank_state,
				});
			}

			let node = TreeLeafNode::new(keys, values).context("tree leaf node is illegal")?;
			TreeNode::Leaf(Arc::new(node))
		}
	};

	Ok(Some(node))
}

async fn package_deprecations_node(
	mut tx: impl AsExecutor,
	hash: &CurrentHash,
) -> anyhow::Result<Option<TreeNode<PackageDeprecationsTree>>> {
	let Some(record) = query!(
		"SELECT kind AS `kind: TreeKind` FROM deprecations_node WHERE blake3 = ?",
		hash.0.as_slice()
	)
	.fetch_optional(tx.as_executor())
	.await?
	else {
		return Ok(None);
	};

	let node = match record.kind {
		TreeKind::Internal => {
			let keys = query!(
				r"
SELECT package.local_name_blake3
FROM deprecations_internal_node_key
INNER JOIN package ON package.scope=deprecations_internal_node_key.scope AND package.pos=deprecations_internal_node_key.package_pos
WHERE deprecations_internal_node_key.node_blake3 = ?
ORDER BY deprecations_internal_node_key.idx",
				hash.0.as_slice()
			)
			.fetch(tx.as_executor())
			.map(|result| {
				let r = result?;
				Ok::<_, anyhow::Error>(PackageDeprecation(LocalNameId::new(
					CurrentHash::new(r.local_name_blake3).context("illegal local name id")?,
				)))
			})
			.try_collect::<ArrayVec<_, _>>()
			.await?;

			let children = query!(
				r"
SELECT child_blake3
FROM deprecations_internal_node_child
WHERE node_blake3 = ?
ORDER BY idx",
				hash.0.as_slice()
			)
			.fetch(tx.as_executor())
			.map(|result| match result {
				Ok(r) => CurrentHash::new(r.child_blake3).context("corrupt child hash"),
				Err(e) => Err(e.into()),
			})
			.try_collect::<ArrayVec<_, _>>()
			.await?;

			let node =
				TreeInternalNode::new(keys, children).context("tree internal node is illegal")?;
			TreeNode::Internal(Arc::new(node))
		}
		TreeKind::Leaf => {
			let mut keys = ArrayVec::new();
			let mut values = ArrayVec::new();

			let mut query = query!(
				r"
SELECT package.local_name_blake3, deprecation_reason.hash, deprecation_reason.plaintext
FROM deprecations_leaf_node_entry
INNER JOIN package ON package.scope=deprecations_leaf_node_entry.scope AND package.pos=deprecations_leaf_node_entry.package_pos
INNER JOIN deprecation_reason ON deprecation_reason.id=deprecations_leaf_node_entry.reason_id
WHERE deprecations_leaf_node_entry.node_blake3 = ?
ORDER BY deprecations_leaf_node_entry.idx",
				hash.0.as_slice()
			)
			.fetch(tx.as_executor());

			while let Some(r) = query.try_next().await? {
				keys.push(PackageDeprecation(LocalNameId::new(
					CurrentHash::new(r.local_name_blake3).context("illegal local name id")?,
				)));

				let reason_hash = r.hash.parse()?;
				let plaintext = Bounded::new(r.plaintext.into())?;

				values.push(PackageDeprecationValue {
					reason: PackageDeprecationReason::new(reason_hash, plaintext)?,
				});
			}

			let node = TreeLeafNode::new(keys, values).context("tree leaf node is illegal")?;
			TreeNode::Leaf(Arc::new(node))
		}
	};

	Ok(Some(node))
}

#[async_trait]
impl ScopeReadRepository for MySqlBackend {
	async fn scope_log_size(&self, scope: &ScopeId) -> anyhow::Result<Option<NonZero<u64>>> {
		scope_log_size(&self.pool, scope).await
	}

	async fn scope_log_entry(
		&self,
		scope: ScopeId,
		pos: u64,
	) -> anyhow::Result<Option<ScopeEntry>> {
		scope_log_entry(&self.pool, scope, pos).await
	}

	async fn scope_state(&self, scope: &ScopeId) -> anyhow::Result<Option<ScopeStateResponse>> {
		scope_state(&self.pool, scope).await
	}

	async fn scope_log_node(
		&self,
		scope: &ScopeId,
		pos: u64,
	) -> anyhow::Result<Option<CurrentHash>> {
		scope_log_node(&self.pool, scope, pos).await
	}

	async fn scope_log_nodes(
		&self,
		scope: &ScopeId,
		positions: Vec<u64>,
	) -> anyhow::Result<Vec<Option<CurrentHash>>> {
		scope_log_nodes(&self.pool, scope, positions).await
	}
}

#[async_trait]
impl ScopeReadRepository for MySqlTransaction {
	async fn scope_log_size(&self, scope: &ScopeId) -> anyhow::Result<Option<NonZero<u64>>> {
		let mut tx = self.tx.lock().await;
		scope_log_size(&mut *tx, scope).await
	}

	async fn scope_log_entry(
		&self,
		scope: ScopeId,
		pos: u64,
	) -> anyhow::Result<Option<ScopeEntry>> {
		let mut tx = self.tx.lock().await;
		scope_log_entry(&mut *tx, scope, pos).await
	}

	async fn scope_state(&self, scope: &ScopeId) -> anyhow::Result<Option<ScopeStateResponse>> {
		let mut tx = self.tx.lock().await;
		scope_state(&mut *tx, scope).await
	}

	async fn scope_log_node(
		&self,
		scope: &ScopeId,
		pos: u64,
	) -> anyhow::Result<Option<CurrentHash>> {
		let mut tx = self.tx.lock().await;
		scope_log_node(&mut *tx, scope, pos).await
	}

	async fn scope_log_nodes(
		&self,
		scope: &ScopeId,
		positions: Vec<u64>,
	) -> anyhow::Result<Vec<Option<CurrentHash>>> {
		let mut tx = self.tx.lock().await;
		scope_log_nodes(&mut *tx, scope, positions).await
	}
}

#[async_trait]
impl ScopeTreeReadRepository<ScopeMembersTree> for MySqlBackend {
	async fn scope_tree_node(
		&self,
		hash: &CurrentHash,
	) -> anyhow::Result<Option<TreeNode<ScopeMembersTree>>> {
		scope_members_node(&self.pool, hash).await
	}
}

#[async_trait]
impl ScopeTreeReadRepository<ScopeMembersTree> for MySqlTransaction {
	async fn scope_tree_node(
		&self,
		hash: &CurrentHash,
	) -> anyhow::Result<Option<TreeNode<ScopeMembersTree>>> {
		let mut tx = self.tx.lock().await;
		scope_members_node(&mut *tx, hash).await
	}
}

#[async_trait]
impl ScopeTreeReadRepository<PackageVersionsTree> for MySqlBackend {
	async fn scope_tree_node(
		&self,
		hash: &CurrentHash,
	) -> anyhow::Result<Option<TreeNode<PackageVersionsTree>>> {
		package_versions_node(&self.pool, hash).await
	}
}

#[async_trait]
impl ScopeTreeReadRepository<PackageVersionsTree> for MySqlTransaction {
	async fn scope_tree_node(
		&self,
		hash: &CurrentHash,
	) -> anyhow::Result<Option<TreeNode<PackageVersionsTree>>> {
		let mut tx = self.tx.lock().await;
		package_versions_node(&mut *tx, hash).await
	}
}

#[async_trait]
impl ScopeTreeReadRepository<PackageDeprecationsTree> for MySqlBackend {
	async fn scope_tree_node(
		&self,
		hash: &CurrentHash,
	) -> anyhow::Result<Option<TreeNode<PackageDeprecationsTree>>> {
		package_deprecations_node(&self.pool, hash).await
	}
}

#[async_trait]
impl ScopeTreeReadRepository<PackageDeprecationsTree> for MySqlTransaction {
	async fn scope_tree_node(
		&self,
		hash: &CurrentHash,
	) -> anyhow::Result<Option<TreeNode<PackageDeprecationsTree>>> {
		let mut tx = self.tx.lock().await;
		package_deprecations_node(&mut *tx, hash).await
	}
}

#[async_trait]
impl ScopeWriteRepository for MySqlTransaction {
	async fn append_scope_log_nodes(
		&mut self,
		scope: &ScopeId,
		pos: u64,
		elems: Vec<CurrentHash>,
	) -> anyhow::Result<()> {
		insert_chunked!(
			&mut **self.tx.get_mut(),
			"scope_log_node",
			["scope", "pos", "blake3"],
			elems.iter().zip(pos..),
			|mut b, (hash, pos)| {
				b.push_bind(scope.hash().0.as_slice())
					.push_bind(pos)
					.push_bind(hash.0.as_slice());
			}
		)
		.await?;

		Ok(())
	}

	async fn set_scope_log_size(&mut self, scope: &ScopeId, size: u64) -> anyhow::Result<()> {
		query!(
			r"
UPDATE scope
INNER JOIN global_log_scope_genesis_entry ON global_log_scope_genesis_entry.pos=scope.genesis_pos
SET scope.log_size = ?
WHERE global_log_scope_genesis_entry.name_blake3 = ?",
			size,
			scope.hash().0.as_slice()
		)
		.execute(&mut **self.tx.get_mut())
		.await?;

		Ok(())
	}

	async fn insert_package_version(
		&mut self,
		scope: &ScopeId,
		pos: u64,
		package_version: &PackageVersion,
		archive_hash: &Hash,
	) -> anyhow::Result<()> {
		let tx = self.tx.get_mut();

		let record = query!(
			r"
SELECT global_log_scope_genesis_entry.pos AS scope_pos, package.pos AS package_pos
FROM global_log_scope_genesis_entry
LEFT JOIN package ON package.scope=global_log_scope_genesis_entry.pos
WHERE global_log_scope_genesis_entry.name_blake3 = ? AND package.local_name_blake3 = ?",
			scope.hash().0.as_slice(),
			package_version.0.hash().0.as_slice(),
		)
		.fetch_one(&mut **tx)
		.await?;

		let package_pos = if let Some(p) = record.package_pos {
			p
		} else {
			query!(
				"INSERT INTO package (scope, pos, local_name_blake3) VALUES (?, ?, ?)",
				record.scope_pos,
				pos,
				package_version.0.hash().0.as_slice(),
			)
			.execute(&mut **tx)
			.await?;

			pos
		};

		query!(
			"INSERT INTO package_version (scope, pos, package_pos, version, archive_hash) VALUES (?, ?, ?, ?, ?)",
			record.scope_pos,
			pos,
			package_pos,
			package_version.1.to_string(),
			archive_hash.to_string(),
		)
		.execute(&mut **tx)
		.await?;

		Ok(())
	}

	async fn insert_scope_entry(&mut self, entry: &ScopeEntry) -> anyhow::Result<()> {
		let tx = self.tx.get_mut();
		let Entry {
			pos,
			published_at,
			payload,
		} = entry;
		let pos = *pos;
		let published_at = jiff_to_chrono(*published_at)?;

		let ScopeOpHeader {
			scope_id,
			prev_hash,
		} = match payload {
			ScopeEntryPayload::User(p) => p.inner().body.header(),
			ScopeEntryPayload::Admin(op) => op.header(),
		};

		let prev = query!(
			r"
SELECT
	global_log_scope_genesis_entry.pos AS `scope_pos`,
	COALESCE(scope_log_entry.owner, global_log_scope_genesis_entry.owner_id) AS `owner`,
	scope_log_entry.scope_members_root_blake3 AS `scope_members_root_blake3?`,
	scope_log_entry.versions_root_blake3 AS `versions_root_blake3?`,
	scope_log_entry.deprecations_root_blake3 AS `deprecations_root_blake3?`
FROM global_log_scope_genesis_entry
LEFT JOIN scope_log_entry ON scope_log_entry.scope=global_log_scope_genesis_entry.pos
WHERE global_log_scope_genesis_entry.name_blake3 = ?
ORDER BY scope_log_entry.pos DESC
LIMIT 1",
			scope_id.hash().0.as_slice(),
		)
		.fetch_one(&mut **tx)
		.await?;

		let scope_pos = prev.scope_pos;

		let insert_root = async |tx, ns, owner, members, versions, deprecations| {
			query!(
				r"
INSERT INTO scope_log_entry (scope, pos, ns, published_at, prev_hash, owner, scope_members_root_blake3, versions_root_blake3, deprecations_root_blake3)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
				scope_pos,
				pos,
				ns,
				published_at,
				prev_hash.to_string(),
				owner,
				members,
				versions,
				deprecations,
			)
			.execute(tx)
			.await
		};

		macro_rules! insert_root {
			(
				$ns:expr,
				$owner:expr
				$(, members = $members:expr)?
				$(, versions = $versions:expr)?
				$(, deprecations = $deprecations:expr)?
			) => {{
				insert_root(
					&mut **tx,
					$ns,
					$owner,
					insert_root!(@root scope_members_root_blake3 ScopeMembersTree => $($members)?),
					insert_root!(@root versions_root_blake3 PackageVersionsTree => $($versions)?),
					insert_root!(@root deprecations_root_blake3 PackageDeprecationsTree => $($deprecations)?),
				).await?;
			}};
			(@root $name:ident $ty:ty => $($given:expr)?) => {{
				let root = None $(.or(Some($given.0.0.as_slice())))?;
				let root = root.or(prev.$name.as_ref().map(|r| r.as_slice()));
				static EMPTY_ROOT: std::sync::OnceLock<Blake3Hash> = std::sync::OnceLock::new();
				root.unwrap_or_else(||
					EMPTY_ROOT.get_or_init(|| {
						use merkle_bplustree::hasher::Hasher;
						<<$ty as TreeConfig>::Hasher as Hasher<<$ty as TreeConfig>::Key, <$ty as TreeConfig>::Value>>::hash_leaf(
							0,
							&<<$ty as TreeConfig>::Hasher as Hasher<<$ty as TreeConfig>::Key, <$ty as TreeConfig>::Value>>::empty_hash()
						)
					})
					.0
					.as_slice()
				)
			}}
		}

		match payload {
			ScopeEntryPayload::User(p) => {
				let UnvalidatedSigned { sig, body } = p.inner();
				let header = body.header();
				let signer_id = Self::insert_public_key(tx, &header.signer).await?;

				let insert_signed = async |tx, kind| {
					query!(
						"INSERT INTO signed_scope_log_entry (scope, pos, op_kind, signer, sig_algorithm, sig) VALUES (?, ?, ?, ?, ?, ?)",
						prev.scope_pos,
						pos,
						kind,
						signer_id,
						Text(sig.kind()),
						sig.data(),
					)
					.execute(tx)
					.await
				};

				macro_rules! insert_signed {
					($kind:expr, $owner:expr $(, $($roots:tt)*)?) => {{
						insert_root!(ScopeNamespace::Signed, $owner $(, $($roots)*)?);

						insert_signed(&mut **tx, $kind).await?;
					}};
				}

				match body {
					UserScopeOp::AddMember(b) => {
						let consent = b.member().inner();
						let member = Self::insert_public_key(tx, &consent.body.consenter).await?;
						let grant_blake3 = Self::insert_grant(tx, &b.grant).await?;

						insert_signed!(
							SignedOpKind::AddMember,
							prev.owner,
							members = &b.scope_members_root
						);

						query!(
							"INSERT INTO signed_add_member_scope_log_entry (scope, pos, member, grant_blake3, consent_algorithm, consent, nonce) VALUES (?, ?, ?, ?, ?, ?, ?)",
							scope_pos,
							pos,
							member,
							grant_blake3.0.as_slice(),
							Text(consent.sig.kind()),
							consent.sig.data(),
							consent.body.nonce.as_bytes().as_slice(),
						)
						.execute(&mut **tx)
						.await?;
					}
					UserScopeOp::UpdateMemberGrant(b) => {
						let member = Self::insert_public_key(tx, &b.member.0).await?;
						let grant_blake3 = Self::insert_grant(tx, &b.grant).await?;

						insert_signed!(
							SignedOpKind::UpdateMemberGrant,
							prev.owner,
							members = &b.scope_members_root
						);

						query!(
							"INSERT INTO signed_update_member_grant_scope_log_entry (scope, pos, member, grant_blake3) VALUES (?, ?, ?, ?)",
							scope_pos,
							pos,
							member,
							grant_blake3.0.as_slice(),
						)
						.execute(&mut **tx)
						.await?;
					}
					UserScopeOp::RotateKey(b) => {
						let consent = b.new_key().inner();
						let new_key = Self::insert_public_key(tx, &consent.body.consenter).await?;

						let owner = if signer_id == prev.owner {
							new_key
						} else {
							prev.owner
						};

						insert_signed!(
							SignedOpKind::RotateKey,
							owner,
							members = &b.scope_members_root
						);

						query!(
							"INSERT INTO signed_rotate_key_scope_log_entry (scope, pos, new_key, consent_algorithm, consent, nonce) VALUES (?, ?, ?, ?, ?, ?)",
							scope_pos,
							pos,
							new_key,
							Text(consent.sig.kind()),
							consent.sig.data(),
							consent.body.nonce.as_bytes().as_slice(),
						)
						.execute(&mut **tx)
						.await?;
					}
					UserScopeOp::RemoveMember(b) => {
						let member = match &b.member {
							Some(m) => Some(Self::insert_public_key(tx, &m.0).await?),
							None => None,
						};

						insert_signed!(
							SignedOpKind::RemoveMember,
							prev.owner,
							members = &b.scope_members_root
						);

						query!(
							"INSERT INTO signed_remove_member_scope_log_entry (scope, pos, member) VALUES (?, ?, ?)",
							scope_pos,
							pos,
							member,
						)
						.execute(&mut **tx)
						.await?;
					}
					UserScopeOp::TransferOwnership(b) => {
						let consent = b.new_owner().inner();
						let new_owner =
							Self::insert_public_key(tx, &consent.body.consenter).await?;

						insert_signed!(SignedOpKind::TransferOwnership, new_owner);

						query!(
							"INSERT INTO signed_transfer_ownership_scope_log_entry (scope, pos, consent_algorithm, consent, nonce) VALUES (?, ?, ?, ?, ?)",
							scope_pos,
							pos,
							Text(consent.sig.kind()),
							consent.sig.data(),
							consent.body.nonce.as_bytes().as_slice(),
						)
						.execute(&mut **tx)
						.await?;
					}
					UserScopeOp::PublishVersion(b) => {
						insert_signed!(
							SignedOpKind::PublishVersion,
							prev.owner,
							versions = &b.versions_root
						);

						query!(
							"INSERT INTO signed_publish_version_scope_log_entry (scope, pos) VALUES (?, ?)",
							scope_pos,
							pos,
						)
						.execute(&mut **tx)
						.await?;
					}
					UserScopeOp::SetYanked(b) => {
						let PackageVersion(pkg, version) = &b.package_version;

						insert_signed!(
							SignedOpKind::SetYanked,
							prev.owner,
							versions = &b.versions_root
						);

						let result = query!(
							r"
INSERT INTO signed_set_yanked_scope_log_entry (scope, pos, version_pos, yanked)
SELECT ?, ?, package_version.pos, ?
FROM package_version
INNER JOIN package ON package.scope=package_version.scope AND package.pos=package_version.package_pos
WHERE package.scope = ? AND package.local_name_blake3 = ? AND package_version.version = ?",
							scope_pos,
							pos,
							b.yanked,
							scope_pos,
							pkg.hash().0.as_slice(),
							version.to_string(),
						)
						.execute(&mut **tx)
						.await?;

						anyhow::ensure!(
							result.rows_affected() == 1,
							"package version {} has no row in this scope",
							b.package_version
						);
					}
					UserScopeOp::SetDeprecation(b) => {
						let package_pos = query!(
							"SELECT pos FROM package WHERE scope = ? AND local_name_blake3 = ?",
							scope_pos,
							b.pkg.hash().0.as_slice(),
						)
						.fetch_one(&mut **tx)
						.await?
						.pos;

						let reason_id = match &b.state {
							Some(state) => Some(
								query!(
									"INSERT INTO deprecation_reason (hash, plaintext) VALUES (?, ?) ON DUPLICATE KEY UPDATE id = LAST_INSERT_ID(id)",
									Text(state.reason.hash()),
									&**state.reason.plaintext(),
								)
								.execute(&mut **tx)
								.await?
								.last_insert_id(),
							),
							None => None,
						};

						insert_signed!(
							SignedOpKind::SetDeprecation,
							prev.owner,
							deprecations = &b.deprecations_root
						);

						query!(
							"INSERT INTO signed_set_deprecation_scope_log_entry (scope, pos, package_pos, reason_id) VALUES (?, ?, ?, ?)",
							scope_pos,
							pos,
							package_pos,
							reason_id,
						)
						.execute(&mut **tx)
						.await?;
					}
				}
			}
			ScopeEntryPayload::Admin(op) => {
				macro_rules! insert_admin {
					($kind:expr, $owner:expr $(, $($roots:tt)*)?) => {{
						insert_root!(ScopeNamespace::Admin, $owner $(, $($roots)*)?);

						query!(
							"INSERT INTO admin_scope_log_entry (scope, pos, op_kind) VALUES (?, ?, ?)",
							scope_pos,
							pos,
							$kind,
						)
						.execute(&mut **tx)
						.await?;
					}};
				}

				match op {
					AdminScopeOp::TransferOwnership(b) => {
						let new_owner = Self::insert_public_key(tx, &b.new_owner).await?;

						insert_admin!(AdminOpKind::TransferOwnership, new_owner);

						query!(
							"INSERT INTO admin_transfer_ownership_scope_log_entry (scope, pos, new_owner) VALUES (?, ?, ?)",
							scope_pos,
							pos,
							new_owner,
						)
						.execute(&mut **tx)
						.await?;
					}
					AdminScopeOp::SetYanked(b) => {
						insert_admin!(
							AdminOpKind::SetYanked,
							prev.owner,
							versions = &b.versions_root
						);

						let result = query!(
							r"
INSERT INTO admin_set_yanked_scope_log_entry (scope, pos, version_pos, yanked)
SELECT ?, ?, package_version.pos, ?
FROM package_version
INNER JOIN package ON package.scope=package_version.scope AND package.pos=package_version.package_pos
WHERE package.scope = ? AND package.local_name_blake3 = ? AND package_version.version = ?",
							scope_pos,
							pos,
							b.yanked,
							scope_pos,
							b.pkg.hash().0.as_slice(),
							b.version.to_string(),
						)
						.execute(&mut **tx)
						.await?;

						anyhow::ensure!(
							result.rows_affected() == 1,
							"package {} version {} has no row in this scope",
							b.pkg,
							b.version
						);
					}
				}
			}
		}

		Ok(())
	}
}

#[async_trait]
impl ScopeTreeWriteRepository<ScopeMembersTree> for MySqlTransaction {
	async fn store_scope_nodes(
		&mut self,
		_scope: &ScopeId,
		nodes: Vec<(HasherOutput<ScopeMembersTree>, TreeNode<ScopeMembersTree>)>,
	) -> anyhow::Result<()> {
		let tx = self.tx.get_mut();

		for (hash, node) in nodes {
			match node {
				TreeNode::Leaf(node) => {
					query!(
						"INSERT INTO scope_members_node (blake3, kind) VALUES (?, ?)",
						hash.0.as_slice(),
						TreeKind::Leaf
					)
					.execute(&mut **tx)
					.await?;

					// TODO: use chunked inserting
					for (idx, (member, value)) in node.keys().iter().zip(node.values()).enumerate()
					{
						let member = Self::insert_public_key(tx, &member.0).await?;
						let grant_blake3 = Self::insert_grant(tx, &value.grant).await?;

						query!(
							"INSERT INTO scope_members_leaf_node_entry (node_blake3, idx, member, grant_blake3) VALUES (?, ?, ?, ?)",
							hash.0.as_slice(),
							idx as u8,
							member,
							grant_blake3.0.as_slice(),
						)
						.execute(&mut **tx)
						.await?;
					}
				}
				TreeNode::Internal(node) => {
					query!(
						"INSERT INTO scope_members_node (blake3, kind) VALUES (?, ?)",
						hash.0.as_slice(),
						TreeKind::Internal
					)
					.execute(&mut **tx)
					.await?;

					// TODO: use chunked inserting
					for (idx, member) in node.keys().iter().enumerate() {
						let member = Self::insert_public_key(tx, &member.0).await?;

						query!(
							"INSERT INTO scope_members_internal_node_key (node_blake3, idx, member) VALUES (?, ?, ?)",
							hash.0.as_slice(),
							idx as u8,
							member,
						)
						.execute(&mut **tx)
						.await?;
					}

					for (idx, child) in node.children().iter().enumerate() {
						query!(
							"INSERT INTO scope_members_internal_node_child (node_blake3, idx, child_blake3) VALUES (?, ?, ?)",
							hash.0.as_slice(),
							idx as u8,
							child.0.as_slice(),
						)
						.execute(&mut **tx)
						.await?;
					}
				}
			}
		}

		Ok(())
	}
}

#[async_trait]
impl ScopeTreeWriteRepository<PackageVersionsTree> for MySqlTransaction {
	async fn store_scope_nodes(
		&mut self,
		scope: &ScopeId,
		nodes: Vec<(
			HasherOutput<PackageVersionsTree>,
			TreeNode<PackageVersionsTree>,
		)>,
	) -> anyhow::Result<()> {
		let tx = self.tx.get_mut();

		for (hash, node) in nodes {
			match node {
				TreeNode::Leaf(node) => {
					query!(
						"INSERT INTO versions_node (blake3, kind) VALUES (?, ?)",
						hash.0.as_slice(),
						TreeKind::Leaf
					)
					.execute(&mut **tx)
					.await?;

					for (idx, (key, value)) in node.keys().iter().zip(node.values()).enumerate() {
						let result = query!(
							r"
INSERT INTO versions_leaf_node_entry (node_blake3, idx, scope, version_pos, yank_state)
SELECT ?, ?, package_version.scope, package_version.pos, ?
FROM package_version
INNER JOIN global_log_scope_genesis_entry ON global_log_scope_genesis_entry.pos=package_version.scope
INNER JOIN package ON package.scope=package_version.scope AND package.pos=package_version.package_pos
WHERE global_log_scope_genesis_entry.name_blake3 = ? AND package.local_name_blake3 = ? AND package_version.version = ?",
							hash.0.as_slice(),
							idx as u8,
							value.yank_state,
							scope.hash().0.as_slice(),
							key.0.hash().0.as_slice(),
							key.1.to_string(),
						)
						.execute(&mut **tx)
						.await?;

						anyhow::ensure!(
							result.rows_affected() == 1,
							"package version {key} has no row in this scope"
						);
					}
				}
				TreeNode::Internal(node) => {
					query!(
						"INSERT INTO versions_node (blake3, kind) VALUES (?, ?)",
						hash.0.as_slice(),
						TreeKind::Internal
					)
					.execute(&mut **tx)
					.await?;

					for (idx, key) in node.keys().iter().enumerate() {
						let result = query!(
							r"
INSERT INTO versions_internal_node_key (node_blake3, idx, scope, version_pos)
SELECT ?, ?, package_version.scope, package_version.pos
FROM package_version
INNER JOIN global_log_scope_genesis_entry ON global_log_scope_genesis_entry.pos=package_version.scope
INNER JOIN package ON package.scope=package_version.scope AND package.pos=package_version.package_pos
WHERE global_log_scope_genesis_entry.name_blake3 = ? AND package.local_name_blake3 = ? AND package_version.version = ?",
							hash.0.as_slice(),
							idx as u8,
							scope.hash().0.as_slice(),
							key.0.hash().0.as_slice(),
							key.1.to_string(),
						)
						.execute(&mut **tx)
						.await?;

						anyhow::ensure!(
							result.rows_affected() == 1,
							"package version {key} has no row in this scope"
						);
					}

					for (idx, child) in node.children().iter().enumerate() {
						query!(
							"INSERT INTO versions_internal_node_child (node_blake3, idx, child_blake3) VALUES (?, ?, ?)",
							hash.0.as_slice(),
							idx as u8,
							child.0.as_slice(),
						)
						.execute(&mut **tx)
						.await?;
					}
				}
			}
		}

		Ok(())
	}
}

#[async_trait]
impl ScopeTreeWriteRepository<PackageDeprecationsTree> for MySqlTransaction {
	async fn store_scope_nodes(
		&mut self,
		scope: &ScopeId,
		nodes: Vec<(
			HasherOutput<PackageDeprecationsTree>,
			TreeNode<PackageDeprecationsTree>,
		)>,
	) -> anyhow::Result<()> {
		let tx = self.tx.get_mut();

		for (hash, node) in nodes {
			match node {
				TreeNode::Leaf(node) => {
					query!(
						"INSERT INTO deprecations_node (blake3, kind) VALUES (?, ?)",
						hash.0.as_slice(),
						TreeKind::Leaf
					)
					.execute(&mut **tx)
					.await?;

					for (idx, (key, value)) in node.keys().iter().zip(node.values()).enumerate() {
						let reason_id = query!(
							"INSERT INTO deprecation_reason (hash, plaintext) VALUES (?, ?) ON DUPLICATE KEY UPDATE id = LAST_INSERT_ID(id)",
							Text(value.reason.hash()),
							&**value.reason.plaintext(),
						)
						.execute(&mut **tx)
						.await?
						.last_insert_id();

						let result = query!(
							r"
INSERT INTO deprecations_leaf_node_entry (node_blake3, idx, scope, package_pos, reason_id)
SELECT ?, ?, package.scope, package.pos, ?
FROM package
INNER JOIN global_log_scope_genesis_entry ON global_log_scope_genesis_entry.pos=package.scope
WHERE global_log_scope_genesis_entry.name_blake3 = ? AND package.local_name_blake3 = ?",
							hash.0.as_slice(),
							idx as u8,
							reason_id,
							scope.hash().0.as_slice(),
							key.0.hash().0.as_slice(),
						)
						.execute(&mut **tx)
						.await?;

						anyhow::ensure!(
							result.rows_affected() == 1,
							"package {} has no row in this scope",
							key.0
						);
					}
				}
				TreeNode::Internal(node) => {
					query!(
						"INSERT INTO deprecations_node (blake3, kind) VALUES (?, ?)",
						hash.0.as_slice(),
						TreeKind::Internal
					)
					.execute(&mut **tx)
					.await?;

					for (idx, key) in node.keys().iter().enumerate() {
						let result = query!(
							r"
INSERT INTO deprecations_internal_node_key (node_blake3, idx, scope, package_pos)
SELECT ?, ?, package.scope, package.pos
FROM package
INNER JOIN global_log_scope_genesis_entry ON global_log_scope_genesis_entry.pos=package.scope
WHERE global_log_scope_genesis_entry.name_blake3 = ? AND package.local_name_blake3 = ?",
							hash.0.as_slice(),
							idx as u8,
							scope.hash().0.as_slice(),
							key.0.hash().0.as_slice(),
						)
						.execute(&mut **tx)
						.await?;

						anyhow::ensure!(
							result.rows_affected() == 1,
							"package {} has no row in this scope",
							key.0
						);
					}

					for (idx, child) in node.children().iter().enumerate() {
						query!(
							"INSERT INTO deprecations_internal_node_child (node_blake3, idx, child_blake3) VALUES (?, ?, ?)",
							hash.0.as_slice(),
							idx as u8,
							child.0.as_slice(),
						)
						.execute(&mut **tx)
						.await?;
					}
				}
			}
		}

		Ok(())
	}
}
