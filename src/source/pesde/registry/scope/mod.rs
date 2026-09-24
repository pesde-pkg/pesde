use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

use crate::{bounded::BoundedBTreeSet, source::pesde::registry::*};

mod op;
pub use op::*;

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

/// Consent data without any validity guarantees
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnvalidatedConsent {
	/// A unique value to make this one-use
	pub nonce: Uuid,
	/// The person consenting
	pub consenter: PublicKey,
}

/// The contextual data along [UnvalidatedConsent] signing covers
#[derive(Debug, Clone, Serialize)]
pub struct ConsentWithContext {
	#[serde(flatten)]
	terms: UnvalidatedConsent,
	scope_id: ScopeId,
	tag: &'static str,
}

impl Signable for ConsentWithContext {
	fn signer(&self) -> &PublicKey {
		&self.terms.consenter
	}
}

/// Wrapper for [ConsentWithContext] that can exist only with verified data
#[derive(Debug, Clone)]
pub struct Consent<T: Tagged>(Signed<ConsentWithContext>, PhantomData<T>);

impl<T: Tagged> Consent<T> {
	/// Returns Ok if the [ScopeId] and [UnvalidatedConsent] matched what is expected
	pub fn new(
		scope_id: ScopeId,
		unvalidated: UnvalidatedSigned<UnvalidatedConsent>,
	) -> Result<Self, BadSignature> {
		let with_context = UnvalidatedSigned {
			sig: unvalidated.sig,
			body: ConsentWithContext {
				terms: unvalidated.body,
				scope_id,
				tag: T::TAG,
			},
		};

		Signed::new(with_context).map(|s| Self(s, PhantomData))
	}

	/// Returns the underlying [UnvalidatedConsent]
	#[must_use]
	pub fn inner(&self) -> UnvalidatedSigned<&UnvalidatedConsent> {
		let UnvalidatedSigned { sig, body } = self.0.inner();
		UnvalidatedSigned::<&UnvalidatedConsent> {
			sig: sig.clone(),
			body: &body.terms,
		}
	}

	/// Returns the consenter
	#[must_use]
	pub fn consenter(&self) -> &PublicKey {
		&self.0.0.body.terms.consenter
	}
}

impl<T: Tagged> Serialize for Consent<T> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.inner().serialize(serializer)
	}
}

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

/// The state of a published package version.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionYankState {
	/// The package is yanked with a normal yank and can be accessed if it has been observed
	Yanked,
	/// The package has been yanked by an admin; it is no longer accessible
	AdminYanked,
}

/// The Merkle B+Tree that contains scope members along their grants
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopeMembersTree(pub CurrentHash);
impl TreeConfig for ScopeMembersTree {
	type Key = PublicKey;
	type Value = ScopeGrant;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 15>;
}

/// The Merkle B+Tree that contains package versions along their state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackageVersionsTree(pub CurrentHash);
impl TreeConfig for PackageVersionsTree {
	type Key = (LocalNameId, PesdeVersionForRegistry);
	type Value = PackageVersionState;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 63>;
}

/// The Merkle B+Tree that contains package deprecations along their reasons
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackageDeprecationsTree(pub CurrentHash);
impl TreeConfig for PackageDeprecationsTree {
	type Key = LocalNameId;
	type Value = Hash;
	type Hasher = CurrentMerkleHasher;
	type Shaper = merkle_bplustree::shape::MaxConstShaper<16, 16, 15>;
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
