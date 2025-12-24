use crate::cli::{
	config::read_config,
	style::{ERROR_STYLE, INFO_STYLE, WARN_STYLE},
};
use anyhow::Context as _;
use futures::StreamExt as _;
use pesde::{
	AuthConfig, DEFAULT_INDEX_NAME, GixUrl, Project,
	engine::{
		EngineKind,
		runtime::{Runtime, RuntimeKind},
	},
	errors::ManifestReadError,
	lockfile::Lockfile,
	manifest::{
		DependencyType, Manifest,
		overrides::{OverrideKey, OverrideSpecifier},
		target::TargetKind,
	},
	names::PackageNames,
	source::{ids::VersionId, specifiers::DependencySpecifiers},
};
use relative_path::RelativePathBuf;
use semver::Version;
use std::{
	collections::{BTreeMap, HashMap, HashSet},
	future::Future,
	path::PathBuf,
	str::FromStr,
};
use tokio::pin;
use tracing::instrument;

pub mod auth;
pub mod commands;
pub mod config;
pub mod install;
pub mod reporters;
pub mod style;
#[cfg(feature = "version-management")]
pub mod version;

pub const PESDE_DIR: &str = concat!(".", env!("CARGO_PKG_NAME"));

fn base_dir() -> anyhow::Result<PathBuf> {
	Ok(match std::env::var("PESDE_HOME") {
		Ok(base) => PathBuf::from(base),
		_ => dirs::home_dir()
			.context("failed to get home directory")?
			.join(PESDE_DIR),
	})
}

pub fn bin_dir() -> anyhow::Result<PathBuf> {
	Ok(base_dir()?.join("bin"))
}

#[cfg(feature = "version-management")]
pub fn engines_dir() -> anyhow::Result<PathBuf> {
	Ok(base_dir()?.join("engines"))
}

pub fn config_path() -> anyhow::Result<PathBuf> {
	Ok(base_dir()?.join("config.toml"))
}

pub fn data_dir() -> anyhow::Result<PathBuf> {
	Ok(base_dir()?.join("data"))
}

pub fn resolve_overrides(
	manifest: &Manifest,
) -> anyhow::Result<BTreeMap<OverrideKey, DependencySpecifiers>> {
	let mut dependencies = None;
	let mut overrides = BTreeMap::new();

	for (key, spec) in &manifest.overrides {
		overrides.insert(
			key.clone(),
			match spec {
				OverrideSpecifier::Specifier(spec) => spec,
				OverrideSpecifier::Alias(alias) => {
					if dependencies.is_none() {
						dependencies = Some(
							manifest
								.all_dependencies()
								.context("failed to get all dependencies")?,
						);
					}

					&dependencies
						.as_ref()
						.and_then(|deps| deps.get(alias))
						.with_context(|| format!("alias `{alias}` not found in manifest"))?
						.0
				}
			}
			.clone(),
		);
	}

	Ok(overrides)
}

#[instrument(skip(project), ret(level = "trace"), level = "debug")]
pub async fn up_to_date_lockfile(project: &Project) -> anyhow::Result<Option<Lockfile>> {
	let manifest = project.deser_manifest().await?;
	let lockfile = match project.deser_lockfile().await {
		Ok(lockfile) => lockfile,
		Err(pesde::errors::LockfileReadError::Io(e))
			if e.kind() == std::io::ErrorKind::NotFound =>
		{
			return Ok(None);
		}
		Err(e) => return Err(e.into()),
	};

	if resolve_overrides(&manifest)? != lockfile.overrides {
		tracing::debug!("overrides are different");
		return Ok(None);
	}

	if manifest.target.kind() != lockfile.target {
		tracing::debug!("target kind is different");
		return Ok(None);
	}

	let specs = lockfile
		.graph
		.iter()
		.filter_map(|(_, node)| {
			node.direct
				.as_ref()
				.map(|(_, spec, source_ty)| (spec, source_ty))
		})
		.collect::<HashSet<_>>();

	let same_dependencies = manifest
		.all_dependencies()
		.context("failed to get all dependencies")?
		.iter()
		.all(|(_, (spec, ty))| specs.contains(&(spec, ty)));

	tracing::debug!("dependencies are the same: {same_dependencies}");

	Ok(same_dependencies.then_some(lockfile))
}

#[derive(Debug, Clone)]
struct VersionedPackageName<V: FromStr = VersionId, N: FromStr = PackageNames>(N, Option<V>);

impl<V: FromStr<Err = E>, E: Into<anyhow::Error>, N: FromStr<Err = F>, F: Into<anyhow::Error>>
	FromStr for VersionedPackageName<V, N>
{
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let mut parts = s.splitn(2, '@');
		let name = parts.next().unwrap();
		let version = parts
			.next()
			.map(FromStr::from_str)
			.transpose()
			.map_err(Into::into)?;

		Ok(VersionedPackageName(
			name.parse().map_err(Into::into)?,
			version,
		))
	}
}

impl VersionedPackageName {
	#[cfg(feature = "patches")]
	fn get(
		self,
		_graph: &pesde::graph::DependencyGraph,
	) -> anyhow::Result<pesde::source::ids::PackageId> {
		unimplemented!()
		// let version_id = if let Some(version) = self.1 {
		// 	version
		// } else {
		// 	let versions = graph
		// 		.keys()
		// 		.filter(|id| *id.name() == self.0)
		// 		.collect::<Vec<_>>();

		// 	match versions.len() {
		// 		0 => anyhow::bail!("package not found"),
		// 		1 => versions[0].version_id().clone(),
		// 		_ => anyhow::bail!(
		// 			"multiple versions found, please specify one of: {}",
		// 			versions
		// 				.iter()
		// 				.map(ToString::to_string)
		// 				.collect::<Vec<_>>()
		// 				.join(", ")
		// 		),
		// 	}
		// };

		// Ok(pesde::source::ids::PackageId::new(self.0, version_id))
	}
}

#[derive(Debug, Clone)]
enum AnyPackageIdentifier<V: FromStr = VersionId, N: FromStr = PackageNames> {
	PackageName(VersionedPackageName<V, N>),
	Url((GixUrl, String)),
	Path(PathBuf),
}

impl<V: FromStr<Err = E>, E: Into<anyhow::Error>, N: FromStr<Err = F>, F: Into<anyhow::Error>>
	FromStr for AnyPackageIdentifier<V, N>
{
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if let Some(s) = s.strip_prefix("gh#") {
			let s = format!("https://github.com/{s}");
			let (repo, rev) = s.split_once('#').context("missing revision")?;

			Ok(AnyPackageIdentifier::Url((repo.parse()?, rev.to_string())))
		} else if let Some(rest) = s.strip_prefix("path:") {
			Ok(AnyPackageIdentifier::Path(rest.into()))
		} else if s.contains(':') {
			let (url, rev) = s.split_once('#').context("missing revision")?;

			Ok(AnyPackageIdentifier::Url((url.parse()?, rev.to_string())))
		} else {
			Ok(AnyPackageIdentifier::PackageName(s.parse()?))
		}
	}
}

pub fn shift_project_dir(project: &Project, pkg_dir: PathBuf) -> Project {
	Project::new(
		pkg_dir,
		Some(project.package_dir()),
		project.data_dir(),
		project.cas_dir(),
		project.auth_config().clone(),
	)
}

pub async fn run_on_workspace_members<F: Future<Output = anyhow::Result<()>>>(
	project: &Project,
	f: impl Fn(Project) -> F,
) -> anyhow::Result<BTreeMap<RelativePathBuf, TargetKind>> {
	// this might seem counterintuitive, but remember that
	// the presence of a workspace dir means that this project is a member of one
	if project.workspace_dir().is_some() {
		return Ok(Default::default());
	}

	let members_future = project.workspace_members(true).await?;
	pin!(members_future);

	let mut results = BTreeMap::<RelativePathBuf, TargetKind>::new();

	while let Some((path, manifest)) = members_future.next().await.transpose()? {
		let relative_path =
			RelativePathBuf::from_path(path.strip_prefix(project.package_dir()).unwrap()).unwrap();

		// don't run on the current workspace root
		if relative_path != "" {
			f(shift_project_dir(project, path)).await?;
		}

		results
			.entry(relative_path)
			.or_insert(manifest.target.kind());
	}

	Ok(results)
}

pub fn display_err(result: anyhow::Result<()>, prefix: &str) {
	if let Err(err) = result {
		eprintln!(
			"{}: {err}\n",
			ERROR_STYLE.apply_to(format!("error{prefix}"))
		);

		let cause = err.chain().skip(1).collect::<Vec<_>>();

		if !cause.is_empty() {
			eprintln!("{}:", ERROR_STYLE.apply_to("caused by"));
			for err in cause {
				eprintln!("\t- {err}");
			}
		}

		let backtrace = err.backtrace();
		match backtrace.status() {
			std::backtrace::BacktraceStatus::Disabled => {
				eprintln!(
					"\n{}: set RUST_BACKTRACE=1 for a backtrace",
					INFO_STYLE.apply_to("help")
				);
			}
			std::backtrace::BacktraceStatus::Captured => {
				eprintln!("\n{}:\n{backtrace}", WARN_STYLE.apply_to("backtrace"));
			}
			_ => {
				eprintln!("\n{}: not captured", WARN_STYLE.apply_to("backtrace"));
			}
		}
	}
}

pub async fn get_index(project: &Project, index: Option<&str>) -> anyhow::Result<GixUrl> {
	let manifest = match project.deser_manifest().await {
		Ok(manifest) => Some(manifest),
		Err(e) => match e {
			ManifestReadError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => None,
			e => return Err(e.into()),
		},
	};

	let index_url = match index {
		Some(index) => index.parse().ok(),
		None => match manifest {
			Some(_) => None,
			None => Some(read_config().await?.default_index),
		},
	};

	if let Some(url) = index_url {
		return Ok(url);
	}

	let index_name = index.unwrap_or(DEFAULT_INDEX_NAME);

	manifest
		.unwrap()
		.indices
		.get(index_name)
		.with_context(|| format!("index {index_name} not found in manifest"))
		.cloned()
}

pub fn dep_type_to_key(dep_type: DependencyType) -> &'static str {
	match dep_type {
		DependencyType::Standard => "dependencies",
		DependencyType::Dev => "dev_dependencies",
		DependencyType::Peer => "peer_dependencies",
	}
}

async fn get_executable_version(engine: EngineKind) -> anyhow::Result<Option<Version>> {
	let output = tokio::process::Command::new(engine.to_string())
		.arg("--version")
		.stdout(std::process::Stdio::piped())
		.output()
		.await;

	let output = match output {
		Ok(output) => output.stdout,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
		Err(e) => return Err(e).context(format!("failed to execute {engine}")),
	};

	let parse = move || {
		let output = String::from_utf8(output)
			.with_context(|| format!("failed to parse {engine} version output"))?;
		let version = output
			.split_once(' ')
			.with_context(|| format!("failed to split {engine} version output"))?
			.1;
		let version = version.trim().trim_start_matches('v');
		let version =
			Version::parse(version).with_context(|| format!("failed to parse {engine} version"))?;

		Ok::<_, anyhow::Error>(version)
	};

	match parse() {
		Ok(version) => Ok(Some(version)),
		Err(err) => {
			let mut cause = vec![];
			let mut source = err.source();
			while let Some(err) = source {
				cause.push(format!("\t- {err}"));
				source = err.source();
			}

			tracing::error!(
				"failed to extract {engine} version:\n{err}{}",
				if cause.is_empty() {
					"".to_string()
				} else {
					"\n".to_string() + &cause.join("\n")
				}
			);
			Ok(None)
		}
	}
}

#[cfg_attr(not(feature = "version-management"), allow(unused_variables))]
pub async fn get_project_engines(
	manifest: &Manifest,
	reqwest: &reqwest::Client,
	auth_config: &AuthConfig,
) -> anyhow::Result<HashMap<EngineKind, Version>> {
	use tokio::task::JoinSet;

	crate::cli::reporters::run_with_reporter(|_, root_progress, reporter| async {
		let root_progress = root_progress;
		#[cfg(feature = "version-management")]
		let reporter = reporter;

		root_progress.set_prefix(format!("{}: ", manifest.target));
		root_progress.reset();
		root_progress.set_message("update engines");

		let tasks = EngineKind::VARIANTS.iter().copied();

		#[cfg(feature = "version-management")]
		let mut tasks = tasks
			.map(|engine| {
				let req = manifest.engines.get(&engine).cloned();
				let reqwest = reqwest.clone();
				let reporter = reporter.clone();
				let auth_config = auth_config.clone();

				async move {
					let Some(req) = req else {
						if let Some(version) =
							version::get_installed_versions(engine).await?.pop_last()
						{
							return Ok(Some((engine, version)));
						}

						return get_executable_version(engine)
							.await
							.map(|opt| opt.map(|ver| (engine, ver)));
					};

					let version = crate::cli::version::get_or_download_engine(
						&reqwest,
						engine,
						req,
						reporter,
						&auth_config,
					)
					.await
					.context("failed to install engine")?
					.1;

					crate::cli::version::make_linker_if_needed(engine)
						.await
						.context("failed to make engine linker")?;

					Ok::<_, anyhow::Error>(Some((engine, version)))
				}
			})
			.collect::<JoinSet<_>>();

		#[cfg(not(feature = "version-management"))]
		let mut tasks = tasks
			.map(|engine| async move {
				get_executable_version(engine)
					.await
					.map(|opt| opt.map(|ver| (engine, ver)))
			})
			.collect::<JoinSet<_>>();

		let mut resolved_engine_versions = HashMap::new();

		while let Some(task) = tasks.join_next().await {
			let Some((engine, version)) = task.unwrap()? else {
				continue;
			};
			resolved_engine_versions.insert(engine, version);
		}

		Ok::<_, anyhow::Error>(resolved_engine_versions)
	})
	.await
}

pub fn compatible_runtime(
	target: TargetKind,
	engines: &HashMap<EngineKind, Version>,
) -> anyhow::Result<Runtime> {
	let runtime = match target {
		TargetKind::Lune => RuntimeKind::Lune,
		TargetKind::Luau => engines
			.keys()
			.find_map(|e| e.as_runtime())
			.context("no runtime available")?,
		TargetKind::Roblox | TargetKind::RobloxServer => {
			anyhow::bail!("roblox targets cannot be ran!")
		}
	};

	Ok(Runtime::new(
		runtime,
		engines
			.get(&runtime.into())
			.with_context(|| format!("{runtime} not available!"))?
			.clone(),
	))
}

pub trait ExecReplace: Sized {
	fn _exec_replace(self) -> std::io::Error;

	fn exec_replace(self) -> ! {
		panic!("failed to replace process: {}", self._exec_replace());
	}
}

#[cfg(unix)]
impl ExecReplace for std::process::Command {
	fn _exec_replace(mut self) -> std::io::Error {
		use std::os::unix::process::CommandExt as _;

		self.exec()
	}
}

#[cfg(windows)]
mod imp {
	use super::ExecReplace;
	use windows::{
		Win32::{Foundation::TRUE, System::Console::SetConsoleCtrlHandler},
		core::BOOL,
	};

	unsafe extern "system" fn ctrlc_handler(_: u32) -> BOOL {
		TRUE
	}

	impl ExecReplace for std::process::Command {
		fn _exec_replace(mut self) -> std::io::Error {
			unsafe {
				if let Err(e) = SetConsoleCtrlHandler(Some(ctrlc_handler), true) {
					return e.into();
				}
			}

			let status = match self.status() {
				Ok(status) => status,
				Err(e) => return e,
			};

			std::process::exit(status.code().unwrap_or(-1))
		}
	}
}

impl ExecReplace for tokio::process::Command {
	fn _exec_replace(self) -> std::io::Error {
		self.into_std()._exec_replace()
	}
}
