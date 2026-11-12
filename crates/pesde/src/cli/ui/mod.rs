use pesde::registry::manifest::DependencyType;

mod command;
pub use command::*;
pub mod public_key;
pub mod specifier;

pub fn dependency_type_to_key(ty: DependencyType) -> &'static str {
	match ty {
		DependencyType::Standard => "dependencies",
		DependencyType::Peer => "peer_dependencies",
		DependencyType::Dev => "dev_dependencies",
	}
}
