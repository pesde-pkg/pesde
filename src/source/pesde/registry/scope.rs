use paste::paste;
use serde::{Deserialize, Serialize};

use crate::{bounded::BoundedBTreeSet, source::pesde::registry::*};

/// Maximum amount of packages a [ScopeGrant] can have
pub const MAX_GRANT_PACKAGES: usize = 255;

/// Maximum length, in characters, of a deprecation reason
pub const MAX_REASON_LEN: usize = 255;

/// The grant a scope member possesses
/// An empty grant means the member can update all packages in the scope
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopeGrant(pub BoundedBTreeSet<LocalNameId, MAX_GRANT_PACKAGES>);

impl ScopeGrant {
	/// Whether this grant allows the member to update this package
	#[must_use]
	pub fn covers(&self, package: &LocalNameId) -> bool {
		if self.0.is_empty() {
			return true;
		}

		self.0.contains(package)
	}
}

macro_rules! ops {
	(
		$(#[$nsmeta:meta])*
		$ns:ident,
		$(
			$(#[$meta:meta])*
			$variant:ident $tag:literal $body:tt,
		)+
	) => {
		paste! {
			$(
				$(#[$meta])*
				#[derive(Debug, Clone, Serialize, Deserialize)]
				#[serde(tag = "kind", rename = $tag)]
				pub struct [< $variant $ns OpBody >] $body

				impl [< $variant $ns OpBody >] {
					/// The tag of this operation to add when serialised
					pub const TAG: &'static str = $tag;
				}

				impl From<[< $variant $ns OpBody >]> for [< $ns Op >] {
					fn from(value: [< $variant $ns OpBody >]) -> Self {
						Self::$variant(value)
					}
				}
			)+

			$(#[$nsmeta])*
			#[derive(Debug, Clone, Serialize, Deserialize)]
			#[serde(untagged)]
			pub enum [< $ns Op >] {
				$(
					$(#[$meta])*
					$variant([< $variant $ns OpBody >])
				),+
			}
		}
	};
}

ops!(
	/// An operation to the scope issued by a regular user
	Signed,
	/// A member is being added
	AddMember "add_member" {
		/// The new member's key
		pub member: PublicKey,
		/// The grant they're being added with
		pub grant: ScopeGrant,
		/// The proof of consent of the new member
		pub consent: Signature,
		/// A value used to prevent playbacks, this is what [consent] signs
		pub nonce: Uuid,
		/// The root of the tree holding scope members. [merkle_bplustree::MerkleBPlusTree]<[ScopeMembersTreeConfig]>
		pub scope_members_root: CurrentHash,
	},
	/// The owner is changing a member's grant
	UpdateMemberGrant "update_member_grant" {
		/// The member's key
		pub member: PublicKey,
		/// The new grant
		pub grant: ScopeGrant,
		/// The root of the tree holding scope members. [merkle_bplustree::MerkleBPlusTree]<[ScopeMembersTreeConfig]>
		pub scope_members_root: CurrentHash,
	},
	/// A member is rotating their key
	RotateKey "rotate_key" {
		/// The key to replace the old key with
		pub new_key: PublicKey,
		/// The proof of possession of the new key
		pub new_key_proof: Signature,
		/// A value used to prevent playbacks, this is what [new_key_proof] signs
		pub nonce: Uuid,
		/// The root of the tree holding scope members. [merkle_bplustree::MerkleBPlusTree]<[ScopeMembersTreeConfig]>
		pub scope_members_root: CurrentHash,
	},
	/// The owner is removing a member
	RemoveMember "remove_member" {
		/// The member being removed. None if the member is removing themselves (signing key is who's leaving)
		#[serde(default, skip_serializing_if = "Option::is_none")]
		pub member: Option<PublicKey>,
		/// The root of the tree holding scope members. [merkle_bplustree::MerkleBPlusTree]<[ScopeMembersTreeConfig]>
		pub scope_members_root: CurrentHash,
	},
	/// The owner is transferring ownership
	TransferOwnership "transfer_ownership" {
		/// The new owner
		pub new_owner: PublicKey,
		/// The proof of consent of the new owner
		pub new_owner_consent: Signature,
		/// A value used to prevent playbacks, this is what [new_owner_consent] signs
		pub nonce: Uuid,
	},
	/// A new version of a package is being published
	PublishVersion "publish_version" {
		/// The package being published
		pub pkg: LocalNameId,
		/// The version being published
		pub version: PesdeVersionForRegistry,
		/// The hash of the archive being published
		pub archive_hash: Hash,
		/// The root of the tree holding package versions. [merkle_bplustree::MerkleBPlusTree]<[PackageVersionsTreeConfig]>
		pub versions_root: CurrentHash,
	},
	/// A package's yank status is being updated
	SetYanked "set_yanked" {
		/// The package being updated
		pub pkg: LocalNameId,
		/// The version being updated
		pub version: PesdeVersionForRegistry,
		/// Whether it is yanked
		pub yanked: bool,
		/// The root of the tree holding package versions. [merkle_bplustree::MerkleBPlusTree]<[PackageVersionsTreeConfig]>
		pub versions_root: CurrentHash,
	},
	/// A package's deprecation status is being updated
	SetDeprecation "set_deprecation" {
		/// The package being updated
		pub pkg: LocalNameId,
		/// The hash of the reason this package is deprecated
		pub reason_hash: Hash,
		/// The root of the tree holding package deprecations. [merkle_bplustree::MerkleBPlusTree]<[PackageDeprecationsTreeConfig]>
		pub deprecations_root: CurrentHash,
	},
);

ops!(
	/// An operation to the scope issued by a registry admin
	Admin,
	/// The admin is transferring ownership
	TransferOwnership "transfer_ownership" {
		/// The new owner
		pub new_owner: PublicKey,
	},
	/// A package's yank status is being updated
	SetYanked "set_yanked" {
		/// The package being updated
		pub pkg: LocalNameId,
		/// The version being updated
		pub version: PesdeVersionForRegistry,
		/// Whether it is yanked
		pub yanked: bool,
		/// The root of the tree holding package versions. [merkle_bplustree::MerkleBPlusTree]<[PackageVersionsTreeConfig]>
		pub versions_root: CurrentHash,
	},
);

/// The fields shared by every scope log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeOpPayload<Op> {
	/// The id of the scope
	pub scope_id: ScopeId,
	/// The hash of the previous entry in this scope's log
	pub prev_hash: Hash,
	/// The operation this entry carries
	pub op: Op,
}

/// A scope log entry payload issued by a normal scope member
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename = "user")]
pub struct UserScopePayload<Op> {
	/// The member signing this
	pub signer: PublicKey,
	/// The scope, prev-hash, and operation this entry carries
	#[serde(flatten)]
	pub op_payload: ScopeOpPayload<Op>,
}

impl<Op> WithSigner for UserScopePayload<Op> {
	fn signer(&self) -> &PublicKey {
		&self.signer
	}
}

/// A scope log entry payload issued by the registry admin
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename = "admin")]
pub struct AdminScopePayload<Op> {
	/// The scope, prev-hash, and operation this entry carries
	#[serde(flatten)]
	pub op_payload: ScopeOpPayload<Op>,
}

/// The payload of an entry in the scope's log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScopeEntryPayload {
	/// An entry issued by a normal user
	User(UserScopeEntryPayload<SignedOp>),
	/// An entry issued by the registry admin
	Admin(AdminScopeEntryPayload<AdminOp>),
}

/// An entry in the scope's chain
pub type ScopeEntry = Entry<ScopeEntryPayload>;
/// An entry payload in the scope's chain, issued by a normal scope member
pub type UserScopeEntryPayload<Op> = Signed<UserScopePayload<Op>>;
/// An entry payload in the scope's chain, issued by the registry admin
pub type AdminScopeEntryPayload<Op> = AdminScopePayload<Op>;

/// An opinionated subset of (Cargo) SemVer.
/// Differences from [Version]:
/// - build metadata is not allowed: it is ambiguous (can't specify it) and overall has little to no purpose
/// - only lowercase ASCII is allowed: while without this requirement versions can be deterministically chosen, they are surprising to users: `1.2.3-hello` is not `1.2.3-Hello`
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PesdeStyleVersion {
	/// [Version::major]
	pub major: u64,
	/// [Version::minor]
	pub minor: u64,
	/// [Version::patch]
	pub patch: u64,
	/// [Version::pre]
	pre: Bounded<Prerelease, 10>,
}
ser_display_deser_fromstr!(PesdeStyleVersion);

impl PesdeStyleVersion {
	/// [Version::pre]
	#[must_use]
	pub fn pre(&self) -> &Prerelease {
		&self.pre
	}
}

impl Display for PesdeStyleVersion {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let pre = if self.pre.is_empty() {
			format_args!("")
		} else {
			format_args!("-{}", self.pre)
		};

		write!(f, "{}.{}.{}{}", self.major, self.minor, self.patch, pre)
	}
}

/// Errors that can occur when parsing a [PesdeStyleVersion] from str
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PesdeStyleVersionFromStrError {
	/// The version was not valid SemVer
	#[error("failed to parse input as semver")]
	SemVer(#[from] semver::Error),

	/// The version contained build metadata
	#[error("pesde style versions mustn't contain build metadata")]
	HasBuildMetadata,

	/// The version's prerelease wasn't lowercase
	#[error("pesde style versions' prereleases must be lowercase")]
	UpperPrerelease,

	/// The version's prerelease was too long
	#[error("pesde style versions' prereleases must be shorter")]
	PrereleaseLength(#[source] crate::bounded::errors::TooLongError),
}

impl FromStr for PesdeStyleVersion {
	type Err = PesdeStyleVersionFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let semver_version = Version::parse(s)?;
		if !semver_version.build.is_empty() {
			return Err(Self::Err::HasBuildMetadata);
		}

		if semver_version.pre.chars().any(|c| c.is_ascii_uppercase()) {
			return Err(Self::Err::UpperPrerelease);
		}

		Ok(Self {
			major: semver_version.major,
			minor: semver_version.minor,
			patch: semver_version.patch,
			pre: Bounded::new(semver_version.pre)
				.map_err(PesdeStyleVersionFromStrError::PrereleaseLength)?,
		})
	}
}

/// Maximum length, in characters, of a serialised version
pub const MAX_VERSION_LEN: usize = 255;

/// A [PesdeStyleVersion] with a maximum length
pub type PesdeVersionForRegistry = Bounded<PesdeStyleVersion, MAX_VERSION_LEN>;

/// Pair of local name and version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VersionedLocalName {
	/// The local name this keys
	pub local_name: LocalName,
	/// The package version this keys
	pub version: PesdeStyleVersion,
}
ser_display_deser_fromstr!(VersionedLocalName);

impl Display for VersionedLocalName {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}@{}", self.local_name, self.version)
	}
}

/// Errors that can occur when parsing a [VersionedLocalName] from str
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum VersionedLocalNameFromStrError {
	/// The input string wasn't in the form of `name@version`
	#[error("`{0}` can't be parsed as `name@version`")]
	BadInput(Box<str>),

	/// The name was invalid
	#[error("failed to parse name")]
	MalformedName(#[from] crate::names::errors::PackageNameError),

	/// The version was invalid
	#[error("failed to parse version")]
	MalformedVersion(#[from] PesdeStyleVersionFromStrError),
}

impl FromStr for VersionedLocalName {
	type Err = VersionedLocalNameFromStrError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let Some((local_name, version)) = s.split_once('@') else {
			return Err(Self::Err::BadInput(s.into()));
		};

		Ok(Self {
			local_name: local_name.parse()?,
			version: version.parse()?,
		})
	}
}

/// The value the map keyed by [PackageVersionsTreeConfig] points to.
/// Monitors must ensure archive_hash is never changed, unlike the mutable [Self::yank_state]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageVersionState {
	/// The hash of the archive containing the package's contents
	pub archive_hash: Hash,
	/// The version's yank status
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub yank_state: Option<VersionYankState>,
}

/// A yank state of a version. In the case of an admin yank, only an admin is able to unyank it
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionYankState {
	/// The package is yanked with a normal yank and can be accessed if it has been observed
	Yanked,
	/// The package has been yanked by an admin; it is no longer accessible
	AdminYanked,
}

/// The tree config for the Merkle B+Tree `scope_members_root` points to
pub struct ScopeMembersTreeConfig;
impl TreeConfig for ScopeMembersTreeConfig {
	type Key = PublicKey;
	type Value = ScopeGrant;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 15>;
}

/// The tree config for the Merkle B+Tree `versions_root` points to
pub struct PackageVersionsTreeConfig;
impl TreeConfig for PackageVersionsTreeConfig {
	type Key = VersionedLocalName;
	type Value = PackageVersionState;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 63>;
}

/// The tree config for the Merkle B+Tree `deprecations_root` points to
pub struct PackageDeprecationsTreeConfig;
impl TreeConfig for PackageDeprecationsTreeConfig {
	type Key = LocalNameId;
	type Value = Hash;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 15>;
}
