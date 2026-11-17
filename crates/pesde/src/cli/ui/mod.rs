use pesde::registry::manifest::DependencyType;

mod command;
pub use command::*;
pub mod public_key;
pub mod signature;
pub mod specifier;

pub fn dependency_type_to_key(ty: DependencyType) -> &'static str {
	match ty {
		DependencyType::Standard => "dependencies",
		DependencyType::Peer => "peer_dependencies",
		DependencyType::Dev => "dev_dependencies",
	}
}

pub fn print_immutable_protocol_warning() {
	eprintln!(
		"IMPORTANT: the pesde registry protocol is backed by cryptographic data structures, including transparency logs"
	);
	eprintln!(
		"data removal capabilities are limited. make sure you understand the implications of this before sending any data"
	);
	eprintln!(
		"make sure to use unique identifiers, such as independent keys, so that they cannot be used to track you"
	);
	eprintln!("visit the documentation of the registry protocol for more information");
}
