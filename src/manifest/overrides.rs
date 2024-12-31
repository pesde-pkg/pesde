use crate::source::specifiers::DependencySpecifiers;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

/// An override key
#[derive(
    Debug, DeserializeFromStr, SerializeDisplay, Clone, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
pub struct OverrideKey(pub Vec<Vec<String>>);

impl FromStr for OverrideKey {
    type Err = errors::OverrideKeyFromStr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let overrides = s
            .split(',')
            .map(|overrides| overrides.split('>').map(ToString::to_string).collect())
            .collect::<Vec<Vec<String>>>();

        if overrides.is_empty() {
            return Err(errors::OverrideKeyFromStr::Empty);
        }

        Ok(Self(overrides))
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for OverrideKey {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "OverrideKey".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": r#"^([a-zA-Z]+(>[a-zA-Z]+)+)(,([a-zA-Z]+(>[a-zA-Z]+)+))*$"#,
        })
    }
}

impl Display for OverrideKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.0
                .iter()
                .map(|overrides| {
                    overrides
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(">")
                })
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

/// A specifier for an override
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum OverrideSpecifier {
    /// A specifier for a dependency
    Specifier(DependencySpecifiers),
    /// An alias for a dependency the current project depends on
    Alias(String),
}

/// Errors that can occur when interacting with override keys
pub mod errors {
    use thiserror::Error;

    /// Errors that can occur when parsing an override key
    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum OverrideKeyFromStr {
        /// The override key is empty
        #[error("empty override key")]
        Empty,
    }
}
