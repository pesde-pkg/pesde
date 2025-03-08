use crate::cli::{
	display_err, run_on_workspace_members,
	style::{ERROR_PREFIX, ERROR_STYLE, SUCCESS_STYLE, WARN_PREFIX},
	up_to_date_lockfile,
};
use anyhow::Context as _;
use async_compression::Level;
use clap::Args;
use console::style;
use fs_err::tokio as fs;
use pesde::{
	manifest::{target::Target, DependencyType},
	matching_globs,
	scripts::ScriptName,
	source::{
		pesde::{specifier::PesdeDependencySpecifier, PesdePackageSource},
		specifiers::DependencySpecifiers,
		traits::{
			GetTargetOptions, PackageRef as _, PackageSource as _, RefreshOptions, ResolveOptions,
		},
		workspace::{
			specifier::{VersionType, VersionTypeOrReq},
			WorkspacePackageSource,
		},
		PackageSources, ADDITIONAL_FORBIDDEN_FILES, IGNORED_DIRS, IGNORED_FILES,
	},
	Project, RefreshedSources, DEFAULT_INDEX_NAME, MANIFEST_FILE_NAME,
};
use relative_path::RelativePath;
use reqwest::{header::AUTHORIZATION, StatusCode};
use semver::VersionReq;
use std::{
	collections::{BTreeMap, BTreeSet},
	path::PathBuf,
	sync::Arc,
};
use tempfile::Builder;
use tokio::{
	io::{AsyncSeekExt as _, AsyncWriteExt as _},
	task::JoinSet,
};

#[derive(Debug, Args, Clone)]
pub struct PublishCommand {
	/// Whether to output a tarball instead of publishing
	#[arg(short, long)]
	dry_run: bool,

	/// Agree to all prompts
	#[arg(short, long)]
	yes: bool,

	/// The index to publish to
	#[arg(short, long, default_value_t = DEFAULT_INDEX_NAME.to_string())]
	index: String,

	/// Whether to skip syntax validation
	#[arg(long)]
	no_verify: bool,
}

impl PublishCommand {
	fn validate_luau_file(&self, name: &str, contents: &str) -> anyhow::Result<()> {
		if self.no_verify {
			return Ok(());
		}

		if let Err(err) = full_moon::parse(contents) {
			eprintln!(
				"{ERROR_PREFIX}: {name} is not a valid Luau file:\n{}",
				err.into_iter()
					.map(|err| format!("\t- {}", ERROR_STYLE.apply_to(err)))
					.collect::<Vec<_>>()
					.join("\n")
			);

			anyhow::bail!("failed to validate Luau file");
		}

		Ok(())
	}

	async fn run_impl(
		self,
		project: &Project,
		reqwest: reqwest::Client,
		refreshed_sources: &RefreshedSources,
	) -> anyhow::Result<()> {
		let mut manifest = project
			.deser_manifest()
			.await
			.context("failed to read manifest")?;

		println!(
			"\n{}\n",
			style(format!(
				"[now publishing {} {}]",
				manifest.name, manifest.target
			))
			.bold()
			.on_color256(235)
		);

		if manifest.private {
			println!(
				"{}",
				ERROR_STYLE.apply_to("package is private, refusing to publish")
			);

			return Ok(());
		}

		if manifest.target.lib_path().is_none()
			&& manifest.target.bin_path().is_none()
			&& manifest.target.scripts().is_none_or(BTreeMap::is_empty)
		{
			anyhow::bail!("no exports found in target");
		}

		if matches!(
			manifest.target,
			Target::Roblox { .. } | Target::RobloxServer { .. }
		) {
			if manifest.target.build_files().is_none_or(BTreeSet::is_empty) {
				anyhow::bail!("no build files found in target");
			}

			match up_to_date_lockfile(project).await? {
				Some(lockfile) => {
					let mut tasks = lockfile
						.graph
						.iter()
						.filter(|(_, node)| node.direct.is_some())
						.map(|(id, node)| {
							let project = project.clone();
							let container_folder = node.container_folder_from_project(
								id,
								&project,
								manifest.target.kind(),
							);

							let id = Arc::new(id.clone());
							let node = node.clone();
							let refreshed_sources = refreshed_sources.clone();

							async move {
								let source = node.pkg_ref.source();
								refreshed_sources
									.refresh(
										&source,
										&RefreshOptions {
											project: project.clone(),
										},
									)
									.await
									.context("failed to refresh source")?;
								let target = source
									.get_target(
										&node.pkg_ref,
										&GetTargetOptions {
											project,
											path: Arc::from(container_folder),
											id,
										},
									)
									.await?;

								Ok::<_, anyhow::Error>(
									target.build_files().is_none()
										&& !matches!(node.resolved_ty, DependencyType::Dev),
								)
							}
						})
						.collect::<JoinSet<_>>();

					while let Some(result) = tasks.join_next().await {
						let result = result
							.unwrap()
							.context("failed to get target of dependency node")?;
						if result {
							anyhow::bail!("roblox packages may not depend on non-roblox packages");
						}
					}
				}
				None => {
					anyhow::bail!("outdated lockfile, please run the install command first")
				}
			}
		}

		let canonical_package_dir = project
			.package_dir()
			.canonicalize()
			.context("failed to canonicalize package directory")?;

		let mut archive = tokio_tar::Builder::new(
			async_compression::tokio::write::GzipEncoder::with_quality(vec![], Level::Best),
		);

		let mut display_build_files: Vec<String> = vec![];

		let (lib_path, bin_path, scripts, target_kind) = (
			manifest
				.target
				.lib_path()
				.map(RelativePath::to_relative_path_buf),
			manifest
				.target
				.bin_path()
				.map(RelativePath::to_relative_path_buf),
			manifest.target.scripts().cloned(),
			manifest.target.kind(),
		);

		let mut roblox_target = match &mut manifest.target {
			Target::Roblox { build_files, .. } => Some(build_files),
			Target::RobloxServer { build_files, .. } => Some(build_files),
			_ => None,
		};

		let mut paths = matching_globs(
			project.package_dir(),
			manifest.includes.iter().map(String::as_str),
			true,
			false,
		)
		.await
		.context("failed to get included files")?;

		if paths.insert(PathBuf::from(MANIFEST_FILE_NAME)) {
			println!("{WARN_PREFIX}: {MANIFEST_FILE_NAME} was not included, adding it");
		}

		if paths.iter().any(|p| p.starts_with(".git")) {
			anyhow::bail!("git directory was included, please remove it");
		}

		if !paths.iter().any(|f| {
			matches!(
				f.to_str().unwrap().to_lowercase().as_str(),
				"readme" | "readme.md" | "readme.txt"
			)
		}) {
			println!("{WARN_PREFIX}: no README file included, consider adding one");
		}

		if !paths.iter().any(|p| p.starts_with("docs")) {
			println!("{WARN_PREFIX}: docs directory not included, consider adding one");
		}

		for path in &paths {
			let Some(file_name) = path.file_name() else {
				continue;
			};

			if ADDITIONAL_FORBIDDEN_FILES.contains(&file_name.to_string_lossy().as_ref()) {
				if file_name == "default.project.json" {
					anyhow::bail!(
						"default.project.json was included at `{}`, this should be generated by the {} script upon dependants installation",
						path.display(),
						ScriptName::RobloxSyncConfigGenerator
					);
				}

				anyhow::bail!(
					"forbidden file {} was included at `{}`",
					file_name.to_string_lossy(),
					path.display()
				);
			}
		}

		for ignored_path in IGNORED_FILES.iter().chain(IGNORED_DIRS.iter()) {
			if paths.iter().any(|p| {
				p.components()
					.any(|ct| ct == std::path::Component::Normal(ignored_path.as_ref()))
			}) {
				anyhow::bail!(
					r"forbidden file {ignored_path} was included.
info: if this was a toolchain manager's manifest file, do not include it due to it possibly messing with user scripts
info: otherwise, the file was deemed unnecessary, if you don't understand why, please contact the maintainers",
				);
			}
		}

		for (name, path) in [("lib path", lib_path), ("bin path", bin_path)] {
			let Some(relative_export_path) = path else {
				continue;
			};

			let export_path = relative_export_path.to_path(&canonical_package_dir);

			let contents = match fs::read_to_string(&export_path).await {
				Ok(contents) => contents,
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
					anyhow::bail!("{name} does not exist");
				}
				Err(e) if e.kind() == std::io::ErrorKind::IsADirectory => {
					anyhow::bail!("{name} must point to a file");
				}
				Err(e) => {
					return Err(e).context(format!("failed to read {name}"));
				}
			};

			let export_path = export_path
				.canonicalize()
				.with_context(|| format!("failed to canonicalize {name}"))?;

			self.validate_luau_file(&format!("file at {name}"), &contents)?;

			let first_part = relative_export_path
				.components()
				.next()
				.with_context(|| format!("{name} must contain at least one part"))?;

			let relative_path::Component::Normal(first_part) = first_part else {
				anyhow::bail!("{name} must be within project directory");
			};

			if paths.insert(
				export_path
					.strip_prefix(&canonical_package_dir)
					.unwrap()
					.to_path_buf(),
			) {
				println!("{WARN_PREFIX}: {name} was not included, adding {relative_export_path}");
			}

			if roblox_target
				.as_mut()
				.is_some_and(|build_files| build_files.insert(first_part.to_string()))
			{
				println!("{WARN_PREFIX}: {name} was not in build files, adding {first_part}");
			}
		}

		if let Some(build_files) = &roblox_target {
			for build_file in build_files.iter() {
				if build_file.eq_ignore_ascii_case(MANIFEST_FILE_NAME) {
					println!(
						"{WARN_PREFIX}: {MANIFEST_FILE_NAME} is in build files, please remove it",
					);

					continue;
				}

				let build_file_path = project.package_dir().join(build_file);

				if !build_file_path.exists() {
					anyhow::bail!("build file {build_file} does not exist");
				}

				if !paths.iter().any(|p| p.starts_with(build_file)) {
					anyhow::bail!("build file {build_file} is not included, please add it");
				}

				if build_file_path.is_file() {
					display_build_files.push(build_file.clone());
				} else {
					display_build_files.push(format!("{build_file}/*"));
				}
			}
		}

		if let Some(scripts) = scripts {
			for (name, path) in scripts {
				let script_path = path.to_path(&canonical_package_dir);

				let contents = match fs::read_to_string(&script_path).await {
					Ok(contents) => contents,
					Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
						anyhow::bail!("script {name} does not exist");
					}
					Err(e) if e.kind() == std::io::ErrorKind::IsADirectory => {
						anyhow::bail!("script {name} must point to a file");
					}
					Err(e) => {
						return Err(e).context(format!("failed to read script {name}"));
					}
				};

				let script_path = script_path
					.canonicalize()
					.with_context(|| format!("failed to canonicalize script {name}"))?;

				self.validate_luau_file(&format!("the `{name}` script"), &contents)?;

				if paths.insert(
					script_path
						.strip_prefix(&canonical_package_dir)
						.unwrap()
						.to_path_buf(),
				) {
					println!("{WARN_PREFIX}: script {name} was not included, adding {path}");
				}
			}
		}

		for relative_path in &paths {
			let path = project.package_dir().join(relative_path);

			if !path.exists() {
				anyhow::bail!("included file `{}` does not exist", path.display());
			}

			let file_name = relative_path
				.file_name()
				.context("failed to get file name")?
				.to_string_lossy()
				.to_string();

			// it'll be included later after transformations, and is guaranteed to be a file
			if file_name.eq_ignore_ascii_case(MANIFEST_FILE_NAME) {
				continue;
			}

			if path.is_file() {
				archive
					.append_file(
						&relative_path,
						fs::File::open(&path)
							.await
							.with_context(|| {
								format!("failed to read `{}`", relative_path.display())
							})?
							.file_mut(),
					)
					.await?;
			}
		}

		for specifier in manifest
			.dependencies
			.values_mut()
			.chain(manifest.dev_dependencies.values_mut())
			.chain(manifest.peer_dependencies.values_mut())
		{
			match specifier {
				DependencySpecifiers::Pesde(specifier) => {
					specifier.index = manifest
						.indices
						.get(&specifier.index)
						.with_context(|| {
							format!("index {} not found in indices field", specifier.index)
						})?
						.to_string();
				}
				#[cfg(feature = "wally-compat")]
				DependencySpecifiers::Wally(specifier) => {
					specifier.index = manifest
						.wally_indices
						.get(&specifier.index)
						.with_context(|| {
							format!("index {} not found in wally_indices field", specifier.index)
						})?
						.to_string();
				}
				DependencySpecifiers::Git(_) => {}
				DependencySpecifiers::Workspace(spec) => {
					let pkg_ref = WorkspacePackageSource
						.resolve(
							spec,
							&ResolveOptions {
								project: project.clone(),
								target: target_kind,
								refreshed_sources: refreshed_sources.clone(),
							},
						)
						.await
						.context("failed to resolve workspace package")?
						.1
						.pop_last()
						.context("no versions found for workspace package")?
						.1;

					let manifest = pkg_ref
						.path
						.to_path(
							project
								.workspace_dir()
								.context("failed to get workspace directory")?,
						)
						.join(MANIFEST_FILE_NAME);
					let manifest = fs::read_to_string(&manifest)
						.await
						.context("failed to read workspace package manifest")?;
					let manifest = toml::from_str::<pesde::manifest::Manifest>(&manifest)
						.context("failed to parse workspace package manifest")?;

					*specifier = DependencySpecifiers::Pesde(PesdeDependencySpecifier {
						name: spec.name.clone(),
						version: match spec.version.clone() {
							VersionTypeOrReq::VersionType(VersionType::Wildcard) => {
								VersionReq::STAR
							}
							VersionTypeOrReq::Req(r) => r,
							VersionTypeOrReq::VersionType(v) => {
								VersionReq::parse(&format!("{v}{}", manifest.version))
									.with_context(|| format!("failed to parse version for {v}"))?
							}
						},
						index: manifest
							.indices
							.get(DEFAULT_INDEX_NAME)
							.context("missing default index in workspace package manifest")?
							.to_string(),
						target: Some(spec.target.unwrap_or(manifest.target.kind())),
					});
				}
				DependencySpecifiers::Path(_) => {
					anyhow::bail!("path dependencies are not allowed in published packages")
				}
			}
		}

		{
			println!(
				"\n{}",
				style("please confirm the following information:").bold()
			);
			println!("name: {}", manifest.name);
			println!("version: {}", manifest.version);
			println!(
				"description: {}",
				manifest.description.as_deref().unwrap_or("(none)")
			);
			println!(
				"license: {}",
				manifest.license.as_deref().unwrap_or("(none)")
			);
			println!(
				"authors: {}",
				if manifest.authors.is_empty() {
					"(none)".to_string()
				} else {
					manifest.authors.join(", ")
				}
			);
			println!(
				"repository: {}",
				manifest
					.repository
					.as_ref()
					.map_or("(none)", url::Url::as_str)
			);

			let roblox_target = roblox_target.is_some_and(|_| true);

			println!("target: {}", manifest.target);
			println!(
				"\tlib path: {}",
				manifest
					.target
					.lib_path()
					.map_or_else(|| "(none)".to_string(), ToString::to_string)
			);

			if roblox_target {
				println!("\tbuild files: {}", display_build_files.join(", "));
			} else {
				println!(
					"\tbin path: {}",
					manifest
						.target
						.bin_path()
						.map_or_else(|| "(none)".to_string(), ToString::to_string)
				);
				println!(
					"\tscripts: {}",
					manifest
						.target
						.scripts()
						.filter(|s| !s.is_empty())
						.map_or_else(
							|| "(none)".to_string(),
							|s| { s.keys().cloned().collect::<Vec<_>>().join(", ") }
						)
				);
			}

			println!(
				"includes: {}",
				paths
					.into_iter()
					.map(|p| p.to_string_lossy().to_string())
					.collect::<Vec<_>>()
					.join(", ")
			);

			if !self.dry_run
				&& !self.yes && !inquire::Confirm::new("is this information correct?").prompt()?
			{
				println!("\n{}", ERROR_STYLE.apply_to("publish aborted"));

				return Ok(());
			}

			println!();
		}

		let temp_path = Builder::new().make(|_| Ok(()))?.into_temp_path();
		let mut temp_manifest = fs::OpenOptions::new()
			.create(true)
			.write(true)
			.truncate(true)
			.read(true)
			.open(temp_path.to_path_buf())
			.await?;

		temp_manifest
			.write_all(
				toml::to_string(&manifest)
					.context("failed to serialize manifest")?
					.as_bytes(),
			)
			.await
			.context("failed to write temp manifest file")?;
		temp_manifest
			.rewind()
			.await
			.context("failed to rewind temp manifest file")?;

		archive
			.append_file(MANIFEST_FILE_NAME, temp_manifest.file_mut())
			.await?;

		let mut encoder = archive
			.into_inner()
			.await
			.context("failed to finish archive")?;
		encoder
			.shutdown()
			.await
			.context("failed to finish archive")?;
		let archive = encoder.into_inner();

		let index_url = manifest
			.indices
			.get(&self.index)
			.with_context(|| format!("missing index {}", self.index))?;
		let source = PesdePackageSource::new(index_url.clone());
		refreshed_sources
			.refresh(
				&PackageSources::Pesde(source.clone()),
				&RefreshOptions {
					project: project.clone(),
				},
			)
			.await
			.context("failed to refresh source")?;
		let config = source
			.config(project)
			.await
			.context("failed to get source config")?;

		if archive.len() > config.max_archive_size {
			anyhow::bail!(
				"archive size exceeds maximum size of {} bytes by {} bytes",
				config.max_archive_size,
				archive.len() - config.max_archive_size
			);
		}

		if self.dry_run {
			fs::write("package.tar.gz", archive).await?;

			println!(
				"{}",
				SUCCESS_STYLE.apply_to("(dry run) package written to package.tar.gz")
			);

			return Ok(());
		}

		let mut request = reqwest
			.post(format!("{}/v1/packages", config.api()))
			.body(archive);

		if let Some(token) = project.auth_config().tokens().get(index_url) {
			tracing::debug!("using token for {index_url}");
			request = request.header(AUTHORIZATION, token);
		}

		let response = request.send().await.context("failed to send request")?;

		let status = response.status();
		let text = response
			.text()
			.await
			.context("failed to get response text")?;
		match status {
			StatusCode::CONFLICT => {
				anyhow::bail!("package version already exists");
			}
			StatusCode::FORBIDDEN => {
				anyhow::bail!("unauthorized to publish under this scope");
			}
			StatusCode::BAD_REQUEST => {
				anyhow::bail!("invalid package: {text}");
			}
			code if !code.is_success() => {
				anyhow::bail!("failed to publish package: {code} ({text})");
			}
			_ => {
				println!("{}", SUCCESS_STYLE.apply_to(text));
			}
		}

		Ok(())
	}

	pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
		let refreshed_sources = RefreshedSources::new();

		let result = self
			.clone()
			.run_impl(&project, reqwest.clone(), &refreshed_sources)
			.await;
		if project.workspace_dir().is_some() {
			return result;
		}

		display_err(result, " occurred publishing workspace root");

		run_on_workspace_members(&project, |project| {
			let reqwest = reqwest.clone();
			let this = self.clone();
			let refreshed_sources = refreshed_sources.clone();
			async move {
				let res = this.run_impl(&project, reqwest, &refreshed_sources).await;
				display_err(res, " occurred publishing workspace member");
				Ok(())
			}
		})
		.await
		.map(|_| ())
	}
}
