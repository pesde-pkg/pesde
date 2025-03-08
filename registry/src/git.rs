use crate::{benv, error::RegistryError, AppState};
use git2::{Remote, Repository, Signature};
use pesde::source::{git_index::GitBasedSource as _, pesde::PesdePackageSource};
use std::collections::HashMap;
use tokio::task::spawn_blocking;

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

const FILE_FILEMODE: i32 = 0o100_644;
const DIR_FILEMODE: i32 = 0o040_000;

pub async fn push_changes(
	app_state: &AppState,
	source: &PesdePackageSource,
	directory: String,
	files: HashMap<String, Vec<u8>>,
	message: String,
) -> Result<(), RegistryError> {
	let path = source.path(&app_state.project);
	let auth_config = app_state.project.auth_config().clone();

	spawn_blocking(move || {
		let repo = Repository::open_bare(path)?;
		let mut oids = HashMap::new();

		let mut remote = repo.find_remote("origin")?;
		let refspec = get_refspec(&repo, &mut remote)?;

		let reference = repo.find_reference(&refspec)?;

		for (name, contents) in files {
			let oid = repo.blob(&contents)?;
			oids.insert(name, oid);
		}

		let old_root_tree = reference.peel_to_tree()?;
		let old_dir_tree = match old_root_tree.get_name(&directory) {
			Some(entry) => Some(repo.find_tree(entry.id())?),
			None => None,
		};

		let mut dir_tree = repo.treebuilder(old_dir_tree.as_ref())?;
		for (file, oid) in oids {
			dir_tree.insert(file, oid, FILE_FILEMODE)?;
		}

		let dir_tree_id = dir_tree.write()?;
		let mut root_tree = repo.treebuilder(Some(&repo.find_tree(old_root_tree.id())?))?;
		root_tree.insert(directory, dir_tree_id, DIR_FILEMODE)?;

		let tree_oid = root_tree.write()?;

		repo.commit(
			Some("HEAD"),
			&signature(),
			&signature(),
			&message,
			&repo.find_tree(tree_oid)?,
			&[&reference.peel_to_commit()?],
		)?;

		let mut push_options = git2::PushOptions::new();
		let mut remote_callbacks = git2::RemoteCallbacks::new();

		let git_creds = auth_config.git_credentials().unwrap();
		remote_callbacks.credentials(|_, _, _| {
			git2::Cred::userpass_plaintext(&git_creds.username, &git_creds.password)
		});

		push_options.remote_callbacks(remote_callbacks);

		remote.push(&[refspec], Some(&mut push_options))?;

		Ok(())
	})
	.await
	.unwrap()
}
