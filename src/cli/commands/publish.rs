use crate::cli::{display_err, run_on_workspace_members, up_to_date_lockfile};
use anyhow::Context;
use async_compression::Level;
use clap::Args;
use colored::Colorize;
use fs_err::tokio as fs;
use pesde::{
    manifest::{target::Target, DependencyType},
    matching_globs,
    scripts::ScriptName,
    source::{
        git_index::GitBasedSource,
        pesde::{specifier::PesdeDependencySpecifier, PesdePackageSource},
        specifiers::DependencySpecifiers,
        traits::{GetTargetOptions, PackageRef, PackageSource, RefreshOptions, ResolveOptions},
        workspace::{
            specifier::{VersionType, VersionTypeOrReq},
            WorkspacePackageSource,
        },
        PackageSources, IGNORED_DIRS, IGNORED_FILES,
    },
    Project, RefreshedSources, DEFAULT_INDEX_NAME, MANIFEST_FILE_NAME, PACKAGES_CONTAINER_NAME,
};
use reqwest::{header::AUTHORIZATION, StatusCode};
use semver::VersionReq;
use std::{path::PathBuf, sync::Arc};
use tempfile::Builder;
use tokio::{
    io::{AsyncSeekExt, AsyncWriteExt},
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
                "{}: {name} is not a valid Luau file:\n{}",
                "error".red().bold(),
                err.into_iter()
                    .map(|err| format!("\t- {err}"))
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
        is_root: bool,
        refreshed_sources: &RefreshedSources,
    ) -> anyhow::Result<()> {
        let mut manifest = project
            .deser_manifest()
            .await
            .context("failed to read manifest")?;

        println!(
            "\n{}\n",
            format!("[now publishing {} {}]", manifest.name, manifest.target)
                .bold()
                .on_bright_black()
        );

        if manifest.private {
            if !is_root {
                println!("{}", "package is private, cannot publish".red().bold());
            }

            return Ok(());
        }

        if manifest.target.lib_path().is_none()
            && manifest.target.bin_path().is_none()
            && manifest.target.scripts().is_none_or(|s| s.is_empty())
        {
            anyhow::bail!("no exports found in target");
        }

        if matches!(
            manifest.target,
            Target::Roblox { .. } | Target::RobloxServer { .. }
        ) {
            if manifest.target.build_files().is_none_or(|f| f.is_empty()) {
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
                            let base_folder = manifest
                                .target
                                .kind()
                                .packages_folder(id.version_id().target());
                            let container_folder = project
                                .package_dir()
                                .join(base_folder)
                                .join(PACKAGES_CONTAINER_NAME)
                                .join(node.container_folder(id));

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
            manifest.target.lib_path().map(|p| p.to_relative_path_buf()),
            manifest.target.bin_path().map(|p| p.to_relative_path_buf()),
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
            manifest.includes.iter().map(|s| s.as_str()),
            true,
            false,
        )
        .await
        .context("failed to get included files")?;

        if paths.insert(PathBuf::from(MANIFEST_FILE_NAME)) {
            println!(
                "{}: {MANIFEST_FILE_NAME} was not included, adding it",
                "warn".yellow().bold()
            );
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
            println!(
                "{}: no README file included, consider adding one",
                "warn".yellow().bold()
            );
        }

        if !paths.iter().any(|p| p.starts_with("docs")) {
            println!(
                "{}: docs directory not included, consider adding one",
                "warn".yellow().bold()
            );
        }

        for path in &paths {
            if path
                .file_name()
                .is_some_and(|n| n == "default.project.json")
            {
                anyhow::bail!(
                    "default.project.json was included at `{}`, this should be generated by the {} script upon dependants installation",
                    path.display(),
                    ScriptName::RobloxSyncConfigGenerator
                );
            }
        }

        for ignored_path in IGNORED_FILES.iter().chain(IGNORED_DIRS.iter()) {
            if paths.iter().any(|p| {
                p.components()
                    .any(|ct| ct == std::path::Component::Normal(ignored_path.as_ref()))
            }) {
                anyhow::bail!(
                    r#"forbidden file {ignored_path} was included.
info: if this was a toolchain manager's manifest file, do not include it due to it possibly messing with user scripts
info: otherwise, the file was deemed unnecessary, if you don't understand why, please contact the maintainers"#,
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

            let first_part = match first_part {
                relative_path::Component::Normal(part) => part,
                _ => anyhow::bail!("{name} must be within project directory"),
            };

            if paths.insert(
                export_path
                    .strip_prefix(&canonical_package_dir)
                    .unwrap()
                    .to_path_buf(),
            ) {
                println!(
                    "{}: {name} was not included, adding {relative_export_path}",
                    "warn".yellow().bold()
                );
            }

            if roblox_target
                .as_mut()
                .is_some_and(|build_files| build_files.insert(first_part.to_string()))
            {
                println!(
                    "{}: {name} was not in build files, adding {first_part}",
                    "warn".yellow().bold()
                );
            }
        }

        if let Some(build_files) = &roblox_target {
            for build_file in build_files.iter() {
                if build_file.eq_ignore_ascii_case(MANIFEST_FILE_NAME) {
                    println!(
                        "{}: {MANIFEST_FILE_NAME} is in build files, please remove it",
                        "warn".yellow().bold()
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
                    println!(
                        "{}: script {name} was not included, adding {path}",
                        "warn".yellow().bold()
                    );
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
                    let index_name = specifier
                        .index
                        .as_deref()
                        .unwrap_or(DEFAULT_INDEX_NAME)
                        .to_string();
                    specifier.index = Some(
                        manifest
                            .indices
                            .get(&index_name)
                            .with_context(|| {
                                format!("index {index_name} not found in indices field")
                            })?
                            .to_string(),
                    );
                }
                #[cfg(feature = "wally-compat")]
                DependencySpecifiers::Wally(specifier) => {
                    let index_name = specifier
                        .index
                        .as_deref()
                        .unwrap_or(DEFAULT_INDEX_NAME)
                        .to_string();
                    specifier.index = Some(
                        manifest
                            .wally_indices
                            .get(&index_name)
                            .with_context(|| {
                                format!("index {index_name} not found in wally_indices field")
                            })?
                            .to_string(),
                    );
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
                            v => VersionReq::parse(&format!("{v}{}", manifest.version))
                                .with_context(|| format!("failed to parse version for {v}"))?,
                        },
                        index: Some(
                            manifest
                                .indices
                                .get(DEFAULT_INDEX_NAME)
                                .context("missing default index in workspace package manifest")?
                                .to_string(),
                        ),
                        target: Some(spec.target.unwrap_or(manifest.target.kind())),
                    });
                }
                DependencySpecifiers::Path(_) => {
                    anyhow::bail!("path dependencies are not allowed in published packages")
                }
            }
        }

        {
            println!("\n{}", "please confirm the following information:".bold());
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
                    .map(|r| r.as_str())
                    .unwrap_or("(none)")
            );

            let roblox_target = roblox_target.is_some_and(|_| true);

            println!("target: {}", manifest.target);
            println!(
                "\tlib path: {}",
                manifest
                    .target
                    .lib_path()
                    .map_or("(none)".to_string(), |p| p.to_string())
            );

            if roblox_target {
                println!("\tbuild files: {}", display_build_files.join(", "));
            } else {
                println!(
                    "\tbin path: {}",
                    manifest
                        .target
                        .bin_path()
                        .map_or("(none)".to_string(), |p| p.to_string())
                );
                println!(
                    "\tscripts: {}",
                    manifest
                        .target
                        .scripts()
                        .filter(|s| !s.is_empty())
                        .map_or("(none)".to_string(), |s| {
                            s.keys().cloned().collect::<Vec<_>>().join(", ")
                        })
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
                && !self.yes
                && !inquire::Confirm::new("is this information correct?").prompt()?
            {
                println!("\n{}", "publish aborted".red().bold());

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

        let deps = manifest.all_dependencies().context("dependency conflict")?;

        if let Some((disallowed, _)) = deps.iter().find(|(_, (spec, _))| match spec {
            DependencySpecifiers::Pesde(spec) => {
                !config.other_registries_allowed.is_allowed_or_same(
                    source.repo_url().clone(),
                    gix::Url::try_from(spec.index.as_deref().unwrap()).unwrap(),
                )
            }
            DependencySpecifiers::Git(spec) => !config.git_allowed.is_allowed(spec.repo.clone()),
            #[cfg(feature = "wally-compat")]
            DependencySpecifiers::Wally(spec) => !config
                .wally_allowed
                .is_allowed(gix::Url::try_from(spec.index.as_deref().unwrap()).unwrap()),
            _ => false,
        }) {
            anyhow::bail!("dependency `{disallowed}` is not allowed on this index");
        }

        if self.dry_run {
            fs::write("package.tar.gz", archive).await?;

            println!(
                "{}",
                "(dry run) package written to package.tar.gz".green().bold()
            );

            return Ok(());
        }

        let mut request = reqwest
            .post(format!("{}/v0/packages", config.api()))
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
                println!("{}", "package version already exists".red().bold());
            }
            StatusCode::FORBIDDEN => {
                println!(
                    "{}",
                    "unauthorized to publish under this scope".red().bold()
                );
            }
            StatusCode::BAD_REQUEST => {
                println!("{}: {text}", "invalid package".red().bold());
            }
            code if !code.is_success() => {
                anyhow::bail!("failed to publish package: {code} ({text})");
            }
            _ => {
                println!("{text}");
            }
        }

        Ok(())
    }

    pub async fn run(self, project: Project, reqwest: reqwest::Client) -> anyhow::Result<()> {
        let refreshed_sources = RefreshedSources::new();

        let result = self
            .clone()
            .run_impl(&project, reqwest.clone(), true, &refreshed_sources)
            .await;
        if project.workspace_dir().is_some() {
            return result;
        } else {
            display_err(result, " occurred publishing workspace root");
        }

        run_on_workspace_members(&project, |project| {
            let reqwest = reqwest.clone();
            let this = self.clone();
            let refreshed_sources = refreshed_sources.clone();
            async move {
                this.run_impl(&project, reqwest, false, &refreshed_sources)
                    .await
            }
        })
        .await
        .map(|_| ())
    }
}
