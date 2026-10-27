pub mod hash;
pub mod signature;

#[doc(hidden)]
pub mod __private {
	/// Implement `Serialize` and `Deserialize` for a type that implements `Display` and `FromStr`
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
	pub use __do_not_use__ser_display_deser_fromstr as ser_display_deser_fromstr;
}
