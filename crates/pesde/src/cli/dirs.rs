use std::path::Path;
use std::sync::LazyLock;

use anyhow::Context as _;

struct Directories {
	config_path: Box<Path>,
	project: directories::ProjectDirs,
}

static DIRECTORIES: LazyLock<anyhow::Result<Directories>> = LazyLock::new(|| {
	let project = directories::ProjectDirs::from("", "", "pesde")
		.context("failed to get project directories")?;

	let directories = Directories {
		config_path: project.config_dir().join("config.toml").into(),
		project,
	};

	Ok(directories)
});
fn directories() -> anyhow::Result<&'static Directories> {
	let directories = LazyLock::force(&DIRECTORIES);
	directories.as_ref().map_err(|e| anyhow::format_err!(e))
}

pub fn config_path() -> anyhow::Result<&'static Path> {
	Ok(directories()?.config_path.as_ref())
}

pub fn cache_dir() -> anyhow::Result<&'static Path> {
	Ok(directories()?.project.cache_dir())
}
