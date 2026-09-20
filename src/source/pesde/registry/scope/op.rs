use std::ops::Deref;

use super::*;
use paste::paste;
use serde::{Deserialize, Serialize};

macro_rules! ops {
	(
		$(#[$containermeta:meta])*
		$container:ident($headerty:ty),
		signed $signed:literal,
		$(
			$(#[$meta:meta])*
			$variant:ident $tag:literal
			$(
				$(#[$consentmeta:meta])*
				consent $consentfield:ident
			)?
			{
				$(
					$(#[$fieldmeta:meta])*
					$field:ident: $fieldty:ty
				),* $(,)?
			},
		)+
	) => {
		paste! {
			$(
				$(#[$meta])*
				#[derive(Debug, Clone, Serialize)]
				pub struct [< $variant $container OpBody >] {
					/// The tag of this operation
					pub kind: OpTag<Self>,
					/// The fields in common between all scope operations of this type
					#[serde(flatten)]
					pub header: $headerty,
					$(
						$(#[$consentmeta])*
						pub $consentfield: Consent<Self>,
					)?
					$(
						$(#[$fieldmeta])*
						pub $field: $fieldty
					),*
				}

				impl [< $variant $container OpBody >] {
					#[allow(dead_code)]
					fn header(&self) -> &$headerty {
						&self.header
					}
				}

				impl Tagged for [< $variant $container OpBody >] {
					const TAG: &'static str = $tag;
				}

				ops!(signed $signed, [< $variant $container OpBody >]);

				impl From<[< $variant $container OpBody >]> for [< $container Op >] {
					fn from(value: [< $variant $container OpBody >]) -> Self {
						Self::$variant(value)
					}
				}

				impl<'de> Deserialize<'de> for [< $variant $container OpBody >] {
					fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
					where
						D: serde::Deserializer<'de>
					{
						#[derive(Deserialize)]
						struct Raw {
							kind: OpTag<[< $variant $container OpBody >]>,
							#[serde(flatten)]
							header: $headerty,
							$(
								$consentfield: UnvalidatedSigned<UnvalidatedConsent>,
							)?
							$(
								$field: $fieldty
							),*
						}

						let raw = Raw::deserialize(deserializer)?;
						Ok(Self {
							kind: raw.kind,
							$(
								$consentfield: Consent::new(raw.header.scope_id.clone(), raw.$consentfield)
									.map_err(serde::de::Error::custom)?,
							)?
							header: raw.header,
							$(
								$field: raw.$field
							),*
						})
					}
				}
			)+

			$(#[$containermeta])*
			#[derive(Debug, Clone, Serialize, Deserialize)]
			#[serde(untagged)]
			pub enum [< $container Op >] {
				$(
					$(#[$meta])*
					$variant([< $variant $container OpBody >])
				),+
			}

			impl [< $container Op >] {
				/// Returns the fields in common between operations of this type
				pub fn header(&self) -> &$headerty {
					match self {
						$(Self::$variant(op) => &op.header),+
					}
				}
			}

			ops!(signed $signed, [< $container Op >]);
		}
	};
	(signed true, $ty:ty) => {
		impl Signable for $ty {
			fn signer(&self) -> &PublicKey {
				&self.header().signer
			}
		}

		impl EntryPayload for Signed<$ty> {}
	};
	(signed false, $ty:ty) => {
		impl EntryPayload for $ty {}
	};
}

/// The fields shared by every scope log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeOpHeader {
	/// The id of the scope
	pub scope_id: ScopeId,
	/// The hash of the previous entry in this scope's log
	pub prev_hash: Hash,
}

/// The fields shared by every user-authored scope log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserScopeOpHeader {
	/// Fields in all scope log entries
	#[serde(flatten)]
	pub common: ScopeOpHeader,
	/// The member signing this operation
	pub signer: PublicKey,
}
impl Deref for UserScopeOpHeader {
	type Target = ScopeOpHeader;

	fn deref(&self) -> &Self::Target {
		&self.common
	}
}

ops!(
	/// An operation to the scope issued by a regular user
	UserScope(UserScopeOpHeader),
	signed true,
	/// A member is being added
	AddMember "add_member"
	/// The member being added
	consent member
	{
		/// The grant they're being added with
		grant: ScopeGrant,
		/// The root of the tree holding scope members
		scope_members_root: ScopeMembersTree,
	},
	/// The owner is changing a member's grant
	UpdateMemberGrant "update_member_grant" {
		/// The member's key
		member: PublicKey,
		/// The new grant
		grant: ScopeGrant,
		/// The root of the tree holding scope members
		scope_members_root: ScopeMembersTree,
	},
	/// A member is rotating their key
	RotateKey "rotate_key"
	/// The key to replace the old key with
	consent new_key
	{
		/// The root of the tree holding scope members
		scope_members_root: ScopeMembersTree,
	},
	/// The owner is removing a member
	RemoveMember "remove_member" {
		/// The member being removed. None if the member is removing themselves (signing key is who's leaving)
		#[serde(default, skip_serializing_if = "Option::is_none")]
		member: Option<PublicKey>,
		/// The root of the tree holding scope members
		scope_members_root: ScopeMembersTree,
	},
	/// The owner is transferring ownership
	TransferOwnership "transfer_ownership"
	/// The new owner
	consent new_owner
	{
	},
	/// A new version of a package is being published
	PublishVersion "publish_version" {
		/// The package being published
		pkg: LocalNameId,
		/// The version being published
		version: PesdeVersionForRegistry,
		/// The hash of the archive being published
		archive_hash: Hash,
		/// The root of the tree holding package versions
		versions_root: PackageVersionsTree,
	},
	/// A package's yank status is being updated
	SetYanked "set_yanked" {
		/// The package being updated
		pkg: LocalNameId,
		/// The version being updated
		version: PesdeVersionForRegistry,
		/// Whether it is yanked
		yanked: bool,
		/// The root of the tree holding package versions
		versions_root: PackageVersionsTree,
	},
	/// A package's deprecation status is being updated
	SetDeprecation "set_deprecation" {
		/// The package being updated
		pkg: LocalNameId,
		/// The hash of the reason this package is deprecated
		reason_hash: Option<Hash>,
		/// The root of the tree holding package deprecations
		deprecations_root: PackageDeprecationsTree,
	},
);

ops!(
	/// An operation to the scope issued by a registry admin
	AdminScope(ScopeOpHeader),
	signed false,
	/// The admin is transferring ownership
	TransferOwnership "admin_transfer_ownership" {
		/// The new owner
		new_owner: PublicKey,
	},
	/// A package's yank status is being updated
	SetYanked "admin_set_yanked" {
		/// The package being updated
		pkg: LocalNameId,
		/// The version being updated
		version: PesdeVersionForRegistry,
		/// Whether it is yanked
		yanked: bool,
		/// The root of the tree holding package versions
		versions_root: PackageVersionsTree,
	},
);

/// The payload of an entry in the scope's log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScopeEntryPayload {
	/// An entry issued by a normal user
	User(Signed<UserScopeOp>),
	/// An entry issued by the registry admin
	Admin(AdminScopeOp),
}
impl EntryPayload for ScopeEntryPayload {}

/// An entry in the scope's chain
pub type ScopeEntry = Entry<ScopeEntryPayload>;
