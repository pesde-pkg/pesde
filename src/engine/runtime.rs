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
pub struct Runtime(RuntimeKind, Version);

impl Runtime {
	/// Creates a [Runtime] from the [RuntimeKind] and [Version]
	#[must_use]
	pub fn new(kind: RuntimeKind, version: Version) -> Self {
		Runtime(kind, version)
	}

	/// Returns the [RuntimeKind] of this Runtime
	#[must_use]
	pub fn kind(&self) -> RuntimeKind {
		self.0
	}

	/// Returns the [Version] of this Runtime
	#[must_use]
	pub fn version(&self) -> &Version {
		&self.1
	}

	/// Prepares a [Command] to execute the given script with the given arguments
	pub fn prepare_command<A: IntoIterator<Item = S> + Debug, S: AsRef<OsStr> + Debug>(
		&self,
		script_path: &OsStr,
		args: A,
	) -> Command {
		let mut command = Command::new(self.0.to_string());

		match self.0 {
			RuntimeKind::Lune => {
				command.arg("run");
				command.arg(script_path);
				if self.1 < Version::new(0, 9, 0) {
					command.arg("--");
				}
				command.args(args);
			}
		}

		command
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
