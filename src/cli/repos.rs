use crate::{
    cli::{config::read_config, home_dir},
    util::authenticate_conn,
};
use anyhow::Context;
use gix::remote::{fetch::Shallow, Direction};
use pesde::Project;
use std::path::Path;

fn update_repo<P: AsRef<Path>>(
    name: &str,
    path: P,
    url: gix::Url,
    project: &Project,
) -> anyhow::Result<()> {
    let path = path.as_ref();
    if path.exists() {
        let repo = gix::open(path).context(format!("failed to open {name} repository"))?;

        let remote = repo
            .find_default_remote(Direction::Fetch)
            .context(format!("missing default remote of {name} repository"))?
            .context(format!(
                "failed to find default remote of {name} repository"
            ))?;

        let mut connection = remote.connect(Direction::Fetch).context(format!(
            "failed to connect to default remote of {name} repository"
        ))?;

        authenticate_conn(&mut connection, project.auth_config());

        let results = connection
            .prepare_fetch(gix::progress::Discard, Default::default())
            .context(format!("failed to prepare {name} repository fetch"))?
            .with_shallow(Shallow::Deepen(1))
            .receive(gix::progress::Discard, &false.into())
            .context(format!("failed to receive new {name} repository contents"))?;

        let remote_ref = results
            .ref_map
            .remote_refs
            .first()
            .context(format!("failed to get remote refs of {name} repository"))?;

        let unpacked = remote_ref.unpack();
        let oid = unpacked
            .1
            .or(unpacked.2)
            .context("couldn't find oid in remote ref")?;

        let tree = repo
            .find_object(oid)
            .context(format!("failed to find {name} repository tree"))?
            .peel_to_tree()
            .context(format!("failed to peel {name} repository object to tree"))?;

        let mut index = gix::index::File::from_state(
            gix::index::State::from_tree(&tree.id, &repo.objects, Default::default()).context(
                format!("failed to create index state from {name} repository tree"),
            )?,
            repo.index_path(),
        );

        let opts = gix::worktree::state::checkout::Options {
            overwrite_existing: true,
            destination_is_initially_empty: false,
            ..Default::default()
        };

        gix::worktree::state::checkout(
            &mut index,
            repo.work_dir().context(format!("{name} repo is bare"))?,
            repo.objects
                .clone()
                .into_arc()
                .context("failed to clone objects")?,
            &gix::progress::Discard,
            &gix::progress::Discard,
            &false.into(),
            opts,
        )
        .context(format!("failed to checkout {name} repository"))?;

        index
            .write(gix::index::write::Options::default())
            .context("failed to write index")?;
    } else {
        fs_err::create_dir_all(path).context(format!("failed to create {name} directory"))?;

        gix::prepare_clone(url, path)
            .context(format!("failed to prepare {name} repository clone"))?
            .with_shallow(Shallow::Deepen(1))
            .fetch_then_checkout(gix::progress::Discard, &false.into())
            .context(format!("failed to fetch and checkout {name} repository"))?
            .0
            .main_worktree(gix::progress::Discard, &false.into())
            .context(format!("failed to set {name} repository as main worktree"))?;
    };

    Ok(())
}

pub fn update_repo_dependencies(project: &Project) -> anyhow::Result<()> {
    let home_dir = home_dir()?;
    let config = read_config()?;

    update_repo(
        "scripts",
        home_dir.join("scripts"),
        config.scripts_repo,
        project,
    )?;

    Ok(())
}
