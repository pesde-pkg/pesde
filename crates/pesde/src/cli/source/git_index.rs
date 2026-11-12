use std::fmt::Debug;
use std::num::NonZero;
use std::path::PathBuf;

use anyhow::Context as _;
use bstr::BString;
use gix::remote::Direction;
use itertools::Itertools as _;
use itertools::Position;
use pesde::Url;
use tokio::task::spawn_blocking;
use tracing::instrument;

const SHALLOW: gix::remote::fetch::Shallow =
	gix::remote::fetch::Shallow::DepthAtRemote(NonZero::new(1).unwrap());

// TODO: use an atomic rename for initial clone
#[instrument(skip_all, level = "debug")]
pub async fn refresh_git_repo(path: PathBuf, repo_url: Url) -> anyhow::Result<()> {
	let context = path.clone();

	if tokio::fs::try_exists(&path)
		.await
		.with_context(|| format!("error checking if path exists: {}", path.display()))?
	{
		spawn_blocking(move || {
			let repo = gix::open_opts(&path, gix::open::Options::isolated())?;
			let remote = repo
				.find_default_remote(Direction::Fetch)
				.context("no default remote found in repository")??;

			remote
				.connect(Direction::Fetch)
				.context("error connecting to remote repository")?
				.prepare_fetch(gix::progress::Discard, Default::default())
				.context("error preparing fetch from remote repository")?
				.with_shallow(SHALLOW)
				.receive(gix::progress::Discard, &false.into())
				.context("error reading from remote repository")?;

			anyhow::Ok(())
		})
		.await
		.unwrap()
		.with_context(|| {
			format!(
				"error refreshing git index of {} at path: {}",
				repo_url,
				context.display()
			)
		})?;

		return Ok(());
	}

	let url = gix::Url::try_from(repo_url.as_str()).context("error parsing git url")?;

	tokio::fs::create_dir_all(&path).await?;

	spawn_blocking(move || {
		gix::clone::PrepareFetch::new(
			url,
			path,
			gix::create::Kind::Bare,
			gix::create::Options::default(),
			gix::open::Options::isolated(),
		)
		.context("error preparing clone of git repository")?
		.with_shallow(SHALLOW)
		.configure_remote(|mut remote| {
			remote.replace_refspecs(["+HEAD:refs/pesde/head"], Direction::Fetch)?;
			Ok(remote)
		})
		.fetch_only(gix::progress::Discard, &false.into())
		.context("error fetching git repository")
		.map(|_| ())
	})
	.await
	.unwrap()
	.with_context(|| {
		format!(
			"error cloning git index of {} at path: {}",
			repo_url,
			context.display()
		)
	})?;

	Ok(())
}

#[instrument(skip(tree), ret, level = "trace")]
pub fn read_file<I: IntoIterator<Item = P> + Debug, P: ToString + PartialEq<gix::bstr::BStr>>(
	tree: &gix::Tree,
	file_path: I,
) -> anyhow::Result<Option<BString>> {
	let mut file_path_str = String::new();

	let entry = match tree.lookup_entry(file_path.into_iter().with_position().map(|(pos, path)| {
		file_path_str.push_str(path.to_string().as_str());
		if !matches!(pos, Position::Last) {
			file_path_str.push('/');
		}

		path
	})) {
		Ok(Some(entry)) => entry,
		Ok(None) => return Ok(None),
		Err(e) => {
			return Err(e).context(format!("error looking up file {file_path_str} in git tree"));
		}
	};

	let object = match entry.object() {
		Ok(object) => object,
		Err(e) => {
			return Err(e).context(format!(
				"error looking up object for file {file_path_str} in git tree"
			));
		}
	};

	let mut blob = object.into_blob();
	let data = std::mem::take(&mut blob.data);
	Ok(Some(BString::new(data)))
}

#[instrument(skip(repo), level = "trace")]
pub(crate) fn root_tree(repo: &gix::Repository) -> anyhow::Result<gix::Tree<'_>> {
	let path = repo.path().to_path_buf();

	repo.find_reference("refs/pesde/head")
		.with_context(|| {
			format!(
				"error finding fetched HEAD ref in repository at {}",
				path.display()
			)
		})?
		.into_fully_peeled_id()
		.with_context(|| {
			format!(
				"error peeling fetched HEAD ref in repository at {}",
				path.display()
			)
		})?
		.object()
		.with_context(|| {
			format!(
				"error reading HEAD object in repository at {}",
				path.display()
			)
		})?
		.peel_to_tree()
		.with_context(|| {
			format!(
				"error peeling HEAD to tree in repository at {}",
				path.display()
			)
		})
}
