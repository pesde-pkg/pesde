pub mod bounded;
pub mod hash;
pub mod manifest;
pub mod registry;
pub mod signature;
mod url;
pub use url::*;

pub use jiff;
pub use merkle_bplustree;
pub use merkleberg;
pub use semver;

#[doc(hidden)]
pub mod __private {
	#[doc(hidden)]
	#[macro_export]
	macro_rules! __do_not_use__ser_display_deser_fromstr {
		($struct_name:ident) => {
			impl ::serde::Serialize for $struct_name {
				fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
				where
					S: ::serde::ser::Serializer,
				{
					serializer.collect_str(self)
				}
			}

			impl<'de> ::serde::Deserialize<'de> for $struct_name {
				fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
				where
					D: ::serde::de::Deserializer<'de>,
				{
					let s = String::deserialize(deserializer)?;
					s.parse().map_err(::serde::de::Error::custom)
				}
			}
		};
	}
	#[doc = "Serialize as Display, Deserialize as FromStr"]
	pub use __do_not_use__ser_display_deser_fromstr as ser_display_deser_fromstr;

	// this doesn't account for ones that WE reserve (context dependent);
	// this checks for OS reserved names to ensure maximum compatibility with all platforms
	#[must_use]
	pub fn is_reserved_file_name(name: &str) -> bool {
		let normalized = name.trim_end_matches([' ', '.']);
		let stem = normalized.split('.').next().unwrap_or(normalized);

		const RESERVED: &[&str] = &[
			"con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
			"com8", "com9", "com¹", "com²", "com³", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6",
			"lpt7", "lpt8", "lpt9", "lpt¹", "lpt²", "lpt³",
		];

		RESERVED
			.iter()
			.any(|reserved| reserved.eq_ignore_ascii_case(stem))
	}
}

mod sealed {
	pub trait Sealed {}
}
