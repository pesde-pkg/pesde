use crate::cli::config::read_config;
use crate::cli::config::write_config;
use anyhow::Context as _;
use clap::Args;
use pesde::DEFAULT_URL_KEY;
use pesde::Subproject;
use pesde::Url;
use pesde::errors::ManifestReadErrorKind;
use pesde::signature::PublicKey;

#[derive(Debug, Args)]
pub struct IdentityCommand {
	/// The registry to use. Defaults to `default`, or the configured default registry if current directory doesn't have a manifest
	#[arg(short, long)]
	registry: Option<String>,

	/// The identity to set. If not passed, the current identity will be printed
	#[arg(index = 1)]
	identity: Option<PublicKey>,
}

impl IdentityCommand {
	pub async fn run(self, subproject: Subproject) -> anyhow::Result<()> {
		let registry = get_registry(&subproject, self.registry.as_deref()).await?;

		let mut config = read_config().await?;

		if let Some(identity) = self.identity {
			config.identities.insert(registry.clone(), identity.clone());
			write_config(&config).await?;
			eprintln!("identity for `{registry}` set to: `{identity}`");
		} else {
			let Some(identity) = config.identities.get(&registry) else {
				eprintln!("no identity set for `{registry}`");
				return Ok(());
			};

			eprintln!("identity for `{registry}`: ");
			println!("{identity}");
		}

		Ok(())
	}
}

async fn get_registry(subproject: &Subproject, registry: Option<&str>) -> anyhow::Result<Url> {
	let manifest = match subproject.deser_manifest().await {
		Ok(manifest) => Some(manifest),
		Err(e) => match e.into_inner() {
			ManifestReadErrorKind::Io(e) if e.kind() == std::io::ErrorKind::NotFound => None,
			e => return Err(e.into()),
		},
	};

	let registry_url = match registry {
		Some(registry) => registry.parse().ok(),
		None => match manifest {
			Some(_) => None,
			None => Some(read_config().await?.default_registry),
		},
	};

	if let Some(url) = registry_url {
		return Ok(url);
	}

	let registry_name = registry.unwrap_or(DEFAULT_URL_KEY);

	manifest
		.unwrap()
		.pesde_registries
		.get(registry_name)
		.with_context(|| format!("registry {registry_name} not found in manifest"))
		.cloned()
}
