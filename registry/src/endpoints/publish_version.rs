use crate::{
    auth::UserId,
    benv,
    error::{Error, ErrorResponse},
    search::update_version,
    storage::StorageImpl,
    AppState,
};
use actix_web::{web, web::Bytes, HttpResponse, Responder};
use async_compression::Level;
use convert_case::{Case, Casing};
use fs_err::tokio as fs;
use futures::{future::join_all, join};
use git2::{Remote, Repository, Signature};
use pesde::{
    manifest::Manifest,
    source::{
        git_index::{read_file, root_tree, GitBasedSource},
        pesde::{DocEntry, DocEntryKind, IndexFile, IndexFileEntry, ScopeInfo, SCOPE_INFO_FILE},
        specifiers::DependencySpecifiers,
        version_id::VersionId,
        IGNORED_DIRS, IGNORED_FILES,
    },
    MANIFEST_FILE_NAME,
};
use sentry::add_breadcrumb;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    io::{Cursor, Write},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn signature<'a>() -> Signature<'a> {
    Signature::now(
        &benv!(required "COMMITTER_GIT_NAME"),
        &benv!(required "COMMITTER_GIT_EMAIL"),
    )
    .unwrap()
}

fn get_refspec(repo: &Repository, remote: &mut Remote) -> Result<String, git2::Error> {
    let upstream_branch_buf = repo.branch_upstream_name(repo.head()?.name().unwrap())?;
    let upstream_branch = upstream_branch_buf.as_str().unwrap();

    let refspec_buf = remote
        .refspecs()
        .find(|r| r.direction() == git2::Direction::Fetch && r.dst_matches(upstream_branch))
        .unwrap()
        .rtransform(upstream_branch)?;
    let refspec = refspec_buf.as_str().unwrap();

    Ok(refspec.to_string())
}

const ADDITIONAL_FORBIDDEN_FILES: &[&str] = &["default.project.json"];

#[derive(Debug, Deserialize, Default)]
struct DocEntryInfo {
    #[serde(default)]
    label: Option<String>,
    #[serde(default, alias = "position")]
    sidebar_position: Option<usize>,
    #[serde(default)]
    collapsed: bool,
}

fn compare_repo_urls(this: &gix::Url, external: &gix::Url) -> bool {
    let this = this.to_bstring().to_string().to_lowercase();
    let external = external.to_bstring().to_string().to_lowercase();

    let this = if this.ends_with(".git") {
        &this[..this.len() - 4]
    } else {
        &this
    };
    
    let external = if external.ends_with(".git") {
        &external[..external.len() - 4]
    } else {
        &external
    };
        
    this == external
}

pub async fn publish_package(
    app_state: web::Data<AppState>,
    bytes: Bytes,
    user_id: web::ReqData<UserId>,
) -> Result<impl Responder, Error> {
    let source = app_state.source.lock().await;
    source.refresh(&app_state.project).await.map_err(Box::new)?;
    let config = source.config(&app_state.project).await?;

    let package_dir = tempfile::tempdir()?;

    {
        let mut decoder = async_compression::tokio::bufread::GzipDecoder::new(Cursor::new(&bytes));
        let mut archive = tokio_tar::Archive::new(&mut decoder);

        archive.unpack(package_dir.path()).await?;
    }

    let mut manifest = None::<Manifest>;
    let mut readme = None::<Vec<u8>>;
    let mut docs = BTreeSet::new();
    let mut docs_pages = HashMap::new();

    let mut read_dir = fs::read_dir(package_dir.path()).await?;
    while let Some(entry) = read_dir.next_entry().await? {
        let file_name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| Error::InvalidArchive("file name contains non UTF-8 characters".into()))?
            .to_string();

        if entry.file_type().await?.is_dir() {
            if IGNORED_DIRS.contains(&file_name.as_str()) {
                return Err(Error::InvalidArchive(format!(
                    "archive contains forbidden directory: {file_name}"
                )));
            }

            if file_name == "docs" {
                let mut stack = vec![(
                    BTreeSet::new(),
                    fs::read_dir(entry.path()).await?,
                    None::<DocEntryInfo>,
                )];

                'outer: while let Some((set, iter, category_info)) = stack.last_mut() {
                    while let Some(entry) = iter.next_entry().await? {
                        let file_name = entry
                            .file_name()
                            .to_str()
                            .ok_or_else(|| {
                                Error::InvalidArchive(
                                    "file name contains non UTF-8 characters".into(),
                                )
                            })?
                            .to_string();

                        if entry.file_type().await?.is_dir() {
                            stack.push((
                                BTreeSet::new(),
                                fs::read_dir(entry.path()).await?,
                                Some(DocEntryInfo {
                                    label: Some(file_name.to_case(Case::Title)),
                                    ..Default::default()
                                }),
                            ));
                            continue 'outer;
                        }

                        if file_name == "_category_.json" {
                            let info = fs::read_to_string(entry.path()).await?;
                            let mut info: DocEntryInfo = serde_json::from_str(&info)?;
                            let old_info = category_info.take();
                            info.label = info.label.or(old_info.and_then(|i| i.label));
                            *category_info = Some(info);
                            continue;
                        }

                        let Some(file_name) = file_name.strip_suffix(".md") else {
                            continue;
                        };

                        let content = fs::read_to_string(entry.path()).await?;
                        let content = content.trim();
                        let hash = format!("{:x}", Sha256::digest(content.as_bytes()));

                        let mut gz = async_compression::tokio::bufread::GzipEncoder::with_quality(
                            Cursor::new(content.as_bytes().to_vec()),
                            Level::Best,
                        );
                        let mut bytes = vec![];
                        gz.read_to_end(&mut bytes).await?;
                        docs_pages.insert(hash.to_string(), bytes);

                        let mut lines = content.lines().peekable();
                        let front_matter = if lines.peek().filter(|l| **l == "---").is_some() {
                            lines.next(); // skip the first `---`

                            let front_matter = lines
                                .by_ref()
                                .take_while(|l| *l != "---")
                                .collect::<Vec<_>>()
                                .join("\n");

                            lines.next(); // skip the last `---`

                            front_matter
                        } else {
                            "".to_string()
                        };

                        let h1 = lines
                            .find(|l| !l.trim().is_empty())
                            .and_then(|l| l.strip_prefix("# "))
                            .map(|s| s.to_string());

                        let info: DocEntryInfo =
                            serde_yaml::from_str(&front_matter).map_err(|_| {
                                Error::InvalidArchive(format!(
                                    "doc {file_name}'s frontmatter isn't valid YAML"
                                ))
                            })?;

                        set.insert(DocEntry {
                            label: info.label.or(h1).unwrap_or(file_name.to_case(Case::Title)),
                            position: info.sidebar_position,
                            kind: DocEntryKind::Page {
                                name: entry
                                    .path()
                                    .strip_prefix(package_dir.path().join("docs"))
                                    .unwrap()
                                    .with_extension("")
                                    .to_str()
                                    .ok_or_else(|| {
                                        Error::InvalidArchive(
                                            "file name contains non UTF-8 characters".into(),
                                        )
                                    })?
                                    // ensure that the path is always using forward slashes
                                    .replace("\\", "/"),
                                hash,
                            },
                        });
                    }

                    // should never be None
                    let (popped, _, category_info) = stack.pop().unwrap();
                    docs = popped;

                    if let Some((set, _, _)) = stack.last_mut() {
                        let category_info = category_info.unwrap_or_default();

                        set.insert(DocEntry {
                            label: category_info.label.unwrap(),
                            position: category_info.sidebar_position,
                            kind: DocEntryKind::Category {
                                items: {
                                    let curr_docs = docs;
                                    docs = BTreeSet::new();
                                    curr_docs
                                },
                                collapsed: category_info.collapsed,
                            },
                        });
                    }
                }
            }

            continue;
        }

        if IGNORED_FILES.contains(&file_name.as_str())
            || ADDITIONAL_FORBIDDEN_FILES.contains(&file_name.as_str())
        {
            return Err(Error::InvalidArchive(format!(
                "archive contains forbidden file: {file_name}"
            )));
        }

        if file_name == MANIFEST_FILE_NAME {
            let content = fs::read_to_string(entry.path()).await?;

            manifest = Some(toml::de::from_str(&content)?);
        } else if file_name
            .to_lowercase()
            .split_once('.')
            .filter(|(file, ext)| *file == "readme" && (*ext == "md" || *ext == "txt"))
            .is_some()
        {
            if readme.is_some() {
                return Err(Error::InvalidArchive(
                    "archive contains multiple readme files".into(),
                ));
            }

            let mut file = fs::File::open(entry.path()).await?;

            let mut gz = async_compression::tokio::write::GzipEncoder::new(vec![]);
            tokio::io::copy(&mut file, &mut gz).await?;
            gz.shutdown().await?;
            readme = Some(gz.into_inner());
        }
    }

    let Some(manifest) = manifest else {
        return Err(Error::InvalidArchive(
            "archive doesn't contain a manifest".into(),
        ));
    };

    add_breadcrumb(sentry::Breadcrumb {
        category: Some("publish".into()),
        message: Some(format!(
            "publish request for {}@{} {}. has readme: {}. docs: {}",
            manifest.name,
            manifest.version,
            manifest.target,
            readme.is_some(),
            docs_pages.len()
        )),
        level: sentry::Level::Info,
        ..Default::default()
    });

    {
        let dependencies = manifest.all_dependencies().map_err(|e| {
            Error::InvalidArchive(format!("manifest has invalid dependencies: {e}"))
        })?;

        for (specifier, _) in dependencies.values() {
            match specifier {
                DependencySpecifiers::Pesde(specifier) => {
                    if specifier
                        .index
                        .as_deref()
                        .filter(|index| match gix::Url::try_from(*index) {
                            Ok(_) if config.other_registries_allowed => true,
                            Ok(url) => compare_repo_urls(source.repo_url(), &url),
                            Err(_) => false,
                        })
                        .is_none()
                    {
                        return Err(Error::InvalidArchive(format!(
                            "invalid index in pesde dependency {specifier}"
                        )));
                    }
                }
                DependencySpecifiers::Wally(specifier) => {
                    if !config.wally_allowed {
                        return Err(Error::InvalidArchive(
                            "wally dependencies are not allowed".into(),
                        ));
                    }

                    if specifier
                        .index
                        .as_ref()
                        .filter(|index| index.parse::<url::Url>().is_ok())
                        .is_none()
                    {
                        return Err(Error::InvalidArchive(format!(
                            "invalid index in wally dependency {specifier}"
                        )));
                    }
                }
                DependencySpecifiers::Git(_) => {
                    if !config.git_allowed {
                        return Err(Error::InvalidArchive(
                            "git dependencies are not allowed".into(),
                        ));
                    }
                }
                DependencySpecifiers::Workspace(_) => {
                    // workspace specifiers are to be transformed into Pesde specifiers by the sender
                    return Err(Error::InvalidArchive(
                        "non-transformed workspace dependency".into(),
                    ));
                }
            }
        }

        let repo = source.repo_git2(&app_state.project)?;
        let gix_repo = gix::open(repo.path())?;
        let gix_tree = root_tree(&gix_repo)?;

        let (scope, name) = manifest.name.as_str();
        let mut oids = vec![];

        match read_file(&gix_tree, [scope, SCOPE_INFO_FILE])? {
            Some(info) => {
                let info: ScopeInfo = toml::de::from_str(&info)?;
                if !info.owners.contains(&user_id.0) {
                    return Ok(HttpResponse::Forbidden().finish());
                }
            }
            None => {
                let scope_info = toml::to_string(&ScopeInfo {
                    owners: BTreeSet::from([user_id.0]),
                })?;

                let mut blob_writer = repo.blob_writer(None)?;
                blob_writer.write_all(scope_info.as_bytes())?;
                oids.push((SCOPE_INFO_FILE, blob_writer.commit()?));
            }
        };

        let mut entries: IndexFile =
            toml::de::from_str(&read_file(&gix_tree, [scope, name])?.unwrap_or_default())?;

        let new_entry = IndexFileEntry {
            target: manifest.target.clone(),
            published_at: chrono::Utc::now(),
            description: manifest.description.clone(),
            license: manifest.license.clone(),
            authors: manifest.authors.clone(),
            repository: manifest.repository.clone(),
            docs,

            dependencies,
        };

        let this_version = entries
            .keys()
            .find(|v_id| *v_id.version() == manifest.version);
        if let Some(this_version) = this_version {
            let other_entry = entries.get(this_version).unwrap();

            // description cannot be different - which one to render in the "Recently published" list?
            // the others cannot be different because what to return from the versions endpoint?
            if other_entry.description != new_entry.description
                || other_entry.license != new_entry.license
                || other_entry.authors != new_entry.authors
                || other_entry.repository != new_entry.repository
            {
                return Ok(HttpResponse::BadRequest().json(ErrorResponse {
                    error: "same version with different description or license already exists"
                        .to_string(),
                }));
            }
        }

        if entries
            .insert(
                VersionId::new(manifest.version.clone(), manifest.target.kind()),
                new_entry.clone(),
            )
            .is_some()
        {
            return Ok(HttpResponse::Conflict().finish());
        }

        let mut remote = repo.find_remote("origin")?;
        let refspec = get_refspec(&repo, &mut remote)?;

        let reference = repo.find_reference(&refspec)?;

        {
            let index_content = toml::to_string(&entries)?;
            let mut blob_writer = repo.blob_writer(None)?;
            blob_writer.write_all(index_content.as_bytes())?;
            oids.push((name, blob_writer.commit()?));
        }

        let old_root_tree = reference.peel_to_tree()?;
        let old_scope_tree = match old_root_tree.get_name(scope) {
            Some(entry) => Some(repo.find_tree(entry.id())?),
            None => None,
        };

        let mut scope_tree = repo.treebuilder(old_scope_tree.as_ref())?;
        for (file, oid) in oids {
            scope_tree.insert(file, oid, 0o100644)?;
        }

        let scope_tree_id = scope_tree.write()?;
        let mut root_tree = repo.treebuilder(Some(&repo.find_tree(old_root_tree.id())?))?;
        root_tree.insert(scope, scope_tree_id, 0o040000)?;

        let tree_oid = root_tree.write()?;

        repo.commit(
            Some("HEAD"),
            &signature(),
            &signature(),
            &format!(
                "add {}@{} {}",
                manifest.name, manifest.version, manifest.target
            ),
            &repo.find_tree(tree_oid)?,
            &[&reference.peel_to_commit()?],
        )?;

        let mut push_options = git2::PushOptions::new();
        let mut remote_callbacks = git2::RemoteCallbacks::new();

        let git_creds = app_state.project.auth_config().git_credentials().unwrap();
        remote_callbacks.credentials(|_, _, _| {
            git2::Cred::userpass_plaintext(&git_creds.username, &git_creds.password)
        });

        push_options.remote_callbacks(remote_callbacks);

        remote.push(&[refspec], Some(&mut push_options))?;

        update_version(&app_state, &manifest.name, new_entry);
    }

    let version_id = VersionId::new(manifest.version.clone(), manifest.target.kind());

    let (a, b, c) = join!(
        app_state
            .storage
            .store_package(&manifest.name, &version_id, bytes.to_vec()),
        join_all(
            docs_pages
                .into_iter()
                .map(|(hash, content)| app_state.storage.store_doc(hash, content)),
        ),
        async {
            if let Some(readme) = readme {
                app_state
                    .storage
                    .store_readme(&manifest.name, &version_id, readme)
                    .await
            } else {
                Ok(())
            }
        }
    );
    a?;
    b.into_iter().collect::<Result<(), _>>()?;
    c?;

    Ok(HttpResponse::Ok().body(format!(
        "published {}@{} {}",
        manifest.name, manifest.version, manifest.target
    )))
}
