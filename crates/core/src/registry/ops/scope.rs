use std::io::Write;
use std::ops::Deref;

use super::super::*;
use paste::paste;
use serde::Deserialize;
use serde::Serialize;

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
				ops!(
					@struct
					[$($(#[$consentmeta])* $consentfield)?]
					[$(#[$meta])*]
					[< $variant $container OpBody >], $headerty,
					{
						$(
							$(#[$fieldmeta])*
							$field: $fieldty
						),*
					}
				);
				impl crate::sealed::Sealed for [< $variant $container OpBody >] {}

				impl [< $variant $container OpBody >] {
					/// Returns the fields in common between all scope operations of this type
					pub fn header(&self) -> &$headerty {
						&self.header
					}
				}

				impl Tagged for [< $variant $container OpBody >] {
					const TAG: &'static str = $tag;
				}

				ops!(@signed $signed, [< $variant $container OpBody >]);

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
							#[serde(flatten)]
							header: $headerty,
							$(
								$(#[$consentmeta])*
								$consentfield: UnvalidatedSigned<UnvalidatedConsent>,
							)?
							$(
								$(#[$fieldmeta])*
								$field: $fieldty
							),*
						}

						let raw = Raw::deserialize(deserializer)?;
						Ok(Self {
							$(
								$consentfield: Consent::new(&raw.header.scope_id, raw.$consentfield)
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
			#[serde(tag = "kind", rename_all = "snake_case")]
			pub enum [< $container Op >] {
				$(
					$(#[$meta])*
					$variant([< $variant $container OpBody >])
				),+
			}
			impl crate::sealed::Sealed for [< $container Op >] {}

			impl [< $container Op >] {
				/// Returns the fields in common between operations of this type
				pub fn header(&self) -> &$headerty {
					match self {
						$(Self::$variant(op) => &op.header),+
					}
				}
			}

			impl Canonical for [< $container Op >] {
				fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
					match self {
						$(Self::$variant(op) => op.write_canonical_to(writer)),+
					}
				}
			}

			ops!(@signed $signed, [< $container Op >]);
		}
	};
	(
		@struct
		[]
		[$(#[$meta:meta])*]
		$name:ident, $headerty:ty,
		{
			$(
				$(#[$fieldmeta:meta])*
				$field:ident: $fieldty:ty
			),*
		}
	) => {
		$(#[$meta])*
		#[derive(Debug, Clone, Serialize)]
		pub struct $name {
			/// The fields in common between all scope operations of this type
			#[serde(flatten)]
			pub header: $headerty,
			$(
				$(#[$fieldmeta])*
				pub $field: $fieldty
			),*
		}
	};
	(
		@struct
		[$(#[$consentmeta:meta])* $consentfield:ident]
		[$(#[$meta:meta])*]
		$name:ident, $headerty:ty,
		{
			$(
				$(#[$fieldmeta:meta])*
				$field:ident: $fieldty:ty
			),*
		}
	) => {
		$(#[$meta])*
		#[derive(Debug, Clone, Serialize)]
		pub struct $name {
			#[serde(flatten)]
			header: $headerty,
			$(#[$consentmeta])*
			$consentfield: Consent<Self>,
			$(
				$(#[$fieldmeta])*
				pub $field: $fieldty
			),*
		}

		impl $name {
			/// Creates this [Self]. Errors if the consent isn't valid.
			pub fn new(
				header: $headerty,
				$consentfield: UnvalidatedSigned<UnvalidatedConsent>,
				$($field: $fieldty),*
			) -> Result<Self, BadSignature> {
				Ok(Self {
					$consentfield: Consent::new(&header.scope_id, $consentfield)?,
					header,
					$($field),*
				})
			}

			$(#[$consentmeta])*
			pub fn $consentfield(&self) -> &Consent<Self> {
				&self.$consentfield
			}
		}
	};
	(@signed true, $ty:ty) => {
		impl Signable for $ty {
			fn signer(&self) -> &PublicKey {
				&self.header().signer
			}

			fn signing_bytes(&self) -> Vec<u8> {
				self.canonical_bytes()
			}
		}

		impl EntryPayload for Signed<$ty> {}
	};
	(@signed false, $ty:ty) => {
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
		member: ScopeMember,
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
		member: Option<ScopeMember>,
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
		/// The package version being published
		package_version: PackageVersion,
		/// The hash of the archive being published
		archive_hash: Hash,
		/// The root of the tree holding package versions
		versions_root: PackageVersionsTree,
	},
	/// A package's yank status is being updated
	SetYanked "set_yanked" {
		/// The package version being published
		package_version: PackageVersion,
		/// Whether it is yanked
		yanked: bool,
		/// The root of the tree holding package versions
		versions_root: PackageVersionsTree,
	},
	/// A package's deprecation status is being updated
	SetDeprecation "set_deprecation" {
		/// The package being updated
		pkg: LocalNameId,
		/// The deprecation state
		state: Option<PackageDeprecationValue>,
		/// The root of the tree holding package deprecations
		deprecations_root: PackageDeprecationsTree,
	},
);

impl Canonical for AddMemberUserScopeOpBody {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		let Self {
			header:
				UserScopeOpHeader {
					common: ScopeOpHeader {
						scope_id,
						prev_hash,
					},
					signer,
				},
			member,
			grant,
			scope_members_root,
		} = self;

		cbor_core::map! {
			"tag" => Self::TAG,
			"scope_id" => scope_id.to_string(),
			"prev_hash" => prev_hash.to_string(),
			"signer" => signer.to_string(),
			"member" => member.as_cbor(),
			"grant" => grant.as_cbor(),
			"scope_members_root" => scope_members_root.0.to_string(),
		}
		.write_to(writer)
	}
}

impl Canonical for UpdateMemberGrantUserScopeOpBody {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		let Self {
			header:
				UserScopeOpHeader {
					common: ScopeOpHeader {
						scope_id,
						prev_hash,
					},
					signer,
				},
			member,
			grant,
			scope_members_root,
		} = self;

		cbor_core::map! {
			"tag" => Self::TAG,
			"scope_id" => scope_id.to_string(),
			"prev_hash" => prev_hash.to_string(),
			"signer" => signer.to_string(),
			"member" => member.0.to_string(),
			"grant" => grant.as_cbor(),
			"scope_members_root" => scope_members_root.0.to_string(),
		}
		.write_to(writer)
	}
}

impl Canonical for RotateKeyUserScopeOpBody {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		let Self {
			header:
				UserScopeOpHeader {
					common: ScopeOpHeader {
						scope_id,
						prev_hash,
					},
					signer,
				},
			new_key,
			scope_members_root,
		} = self;

		cbor_core::map! {
			"tag" => Self::TAG,
			"scope_id" => scope_id.to_string(),
			"prev_hash" => prev_hash.to_string(),
			"signer" => signer.to_string(),
			"new_key" => new_key.as_cbor(),
			"scope_members_root" => scope_members_root.0.to_string(),
		}
		.write_to(writer)
	}
}

impl Canonical for RemoveMemberUserScopeOpBody {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		let Self {
			header:
				UserScopeOpHeader {
					common: ScopeOpHeader {
						scope_id,
						prev_hash,
					},
					signer,
				},
			member,
			scope_members_root,
		} = self;

		cbor_core::map! {
			"tag" => Self::TAG,
			"scope_id" => scope_id.to_string(),
			"prev_hash" => prev_hash.to_string(),
			"signer" => signer.to_string(),
			"member" => member.as_ref().map_or(const { cbor_core::Value::null() }, |m| cbor_core::Value::TextString(m.0.to_string().into())),
			"scope_members_root" => scope_members_root.0.to_string(),
		}
		.write_to(writer)
	}
}

impl Canonical for TransferOwnershipUserScopeOpBody {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		let Self {
			header:
				UserScopeOpHeader {
					common: ScopeOpHeader {
						scope_id,
						prev_hash,
					},
					signer,
				},
			new_owner,
		} = self;

		cbor_core::map! {
			"tag" => Self::TAG,
			"scope_id" => scope_id.to_string(),
			"prev_hash" => prev_hash.to_string(),
			"signer" => signer.to_string(),
			"new_owner" => new_owner.as_cbor(),
		}
		.write_to(writer)
	}
}

impl Canonical for PublishVersionUserScopeOpBody {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		let Self {
			header:
				UserScopeOpHeader {
					common: ScopeOpHeader {
						scope_id,
						prev_hash,
					},
					signer,
				},
			package_version,
			archive_hash,
			versions_root,
		} = self;

		cbor_core::map! {
			"tag" => Self::TAG,
			"scope_id" => scope_id.to_string(),
			"prev_hash" => prev_hash.to_string(),
			"signer" => signer.to_string(),
			"pkg" => package_version.0.to_string(),
			"version" => package_version.1.to_string(),
			"archive_hash" => archive_hash.to_string(),
			"versions_root" => versions_root.0.to_string(),
		}
		.write_to(writer)
	}
}

impl Canonical for SetYankedUserScopeOpBody {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		let Self {
			header:
				UserScopeOpHeader {
					common: ScopeOpHeader {
						scope_id,
						prev_hash,
					},
					signer,
				},
			package_version,
			yanked,
			versions_root,
		} = self;

		cbor_core::map! {
			"tag" => Self::TAG,
			"scope_id" => scope_id.to_string(),
			"prev_hash" => prev_hash.to_string(),
			"signer" => signer.to_string(),
			"pkg" => package_version.0.to_string(),
			"version" => package_version.1.to_string(),
			"yanked" => *yanked,
			"versions_root" => versions_root.0.to_string(),
		}
		.write_to(writer)
	}
}

impl Canonical for SetDeprecationUserScopeOpBody {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		let Self {
			header:
				UserScopeOpHeader {
					common: ScopeOpHeader {
						scope_id,
						prev_hash,
					},
					signer,
				},
			pkg,
			state,
			deprecations_root,
		} = self;

		cbor_core::map! {
			"tag" => Self::TAG,
			"scope_id" => scope_id.to_string(),
			"prev_hash" => prev_hash.to_string(),
			"signer" => signer.to_string(),
			"pkg" => pkg.to_string(),
			"reason_hash" => state.as_ref().map_or(const { cbor_core::Value::null() }, |s| cbor_core::Value::TextString(s.reason.hash().to_string().into())),
			"deprecations_root" => deprecations_root.0.to_string(),
		}
		.write_to(writer)
	}
}

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
		version: PesdeStyleVersion,
		/// Whether it is yanked
		yanked: bool,
		/// The root of the tree holding package versions
		versions_root: PackageVersionsTree,
	},
);

impl Canonical for TransferOwnershipAdminScopeOpBody {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		let Self {
			header: ScopeOpHeader {
				scope_id,
				prev_hash,
			},
			new_owner,
		} = self;

		cbor_core::map! {
			"tag" => Self::TAG,
			"scope_id" => scope_id.to_string(),
			"prev_hash" => prev_hash.to_string(),
			"new_owner" => new_owner.to_string(),
		}
		.write_to(writer)
	}
}

impl Canonical for SetYankedAdminScopeOpBody {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		let Self {
			header: ScopeOpHeader {
				scope_id,
				prev_hash,
			},
			pkg,
			version,
			yanked,
			versions_root,
		} = self;

		cbor_core::map! {
			"tag" => Self::TAG,
			"scope_id" => scope_id.to_string(),
			"prev_hash" => prev_hash.to_string(),
			"pkg" => pkg.to_string(),
			"version" => version.to_string(),
			"yanked" => *yanked,
			"versions_root" => versions_root.0.to_string(),
		}
		.write_to(writer)
	}
}

/// The payload of an entry in the scope's log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "ns", rename_all = "snake_case")]
pub enum ScopeEntryPayload {
	/// An entry issued by a normal user
	User(Signed<UserScopeOp>),
	/// An entry issued by the registry admin
	Admin(AdminScopeOp),
}
impl crate::sealed::Sealed for ScopeEntryPayload {}
impl EntryPayload for ScopeEntryPayload {}

impl Canonical for ScopeEntryPayload {
	fn write_canonical_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
		match self {
			Self::User(p) => p.write_canonical_to(writer),
			Self::Admin(p) => p.write_canonical_to(writer),
		}
	}
}

/// An entry in the scope's chain
pub type ScopeEntry = Entry<ScopeEntryPayload>;
