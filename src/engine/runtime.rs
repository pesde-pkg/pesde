use std::{
	collections::HashMap,
	ffi::OsStr,
	fmt::{Debug, Display},
};

use semver::Version;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::EngineKind;

pub(crate) type Engines = HashMap<EngineKind, Version>;

/// A runtime
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
	/// The Lune runtime
	Lune,
}

impl Display for RuntimeKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Lune => write!(f, "lune"),
		}
	}
}

/// Supported runtimes
#[derive(Debug, Clone)]
pub enum Runtime {
	/// The [EngineKind::Lune] runtime
	Lune(Version),
}

impl Runtime {
	/// Prepares a [Command] to execute the given script with the given arguments
	pub fn prepare_command<A: IntoIterator<Item = S> + Debug, S: AsRef<OsStr> + Debug>(
		&self,
		script_path: &OsStr,
		args: A,
	) -> Command {
		let mut command = Command::new(match self {
			Self::Lune(..) => "lune",
		});

		match self {
			Self::Lune(version) => {
				command.arg("run");
				command.arg(script_path);
				if *version < Version::new(0, 9, 0) {
					command.arg("--");
				}
				command.args(args);
			}
		}

		command
	}
}

impl Runtime {
	/// Creates a [Runtime] from the [RuntimeKind] and [Version]
	#[must_use]
	pub fn new(kind: RuntimeKind, version: Version) -> Self {
		match kind {
			RuntimeKind::Lune => Runtime::Lune(version),
		}
	}
}

impl EngineKind {
	/// Returns this engine as a [RuntimeKind], if it is one
	#[must_use]
	pub fn as_runtime(self) -> Option<RuntimeKind> {
		Some(match self {
			EngineKind::Pesde => return None,
			EngineKind::Lune => RuntimeKind::Lune,
		})
	}
}

impl From<RuntimeKind> for EngineKind {
	fn from(value: RuntimeKind) -> Self {
		match value {
			RuntimeKind::Lune => EngineKind::Lune,
		}
	}
}
