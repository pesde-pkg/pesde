use crate::{
	download::DownloadGraphOptions,
	graph::{
		DependencyGraph, DependencyGraphNode, DependencyGraphNodeWithTarget,
		DependencyGraphWithTarget,
	},
	manifest::{target::TargetKind, DependencyType},
	reporters::DownloadsReporter,
	source::{
		ids::PackageId,
		traits::{GetTargetOptions, PackageRef, PackageSource},
	},
	Project, RefreshedSources,
};
use fs_err::tokio as fs;
use futures::TryStreamExt;
use std::{
	collections::{BTreeMap, HashMap},
	convert::Infallible,
	future::{self, Future},
	num::NonZeroUsize,
	sync::Arc,
};
use tokio::{pin, task::JoinSet};
use tracing::{instrument, Instrument};

/// Hooks to perform actions after certain events during download and linking.
#[allow(unused_variables)]
pub trait DownloadAndLinkHooks {
	/// The error type for the hooks.
	type Error: std::error::Error + Send + Sync + 'static;

	/// Called after scripts have been downloaded. The `downloaded_graph`
	/// contains all downloaded packages.
	fn on_scripts_downloaded(
		&self,
		graph: &DependencyGraphWithTarget,
	) -> impl Future<Output = Result<(), Self::Error>> + Send {
		future::ready(Ok(()))
	}

	/// Called after binary dependencies have been downloaded. The
	/// `downloaded_graph` contains all downloaded packages.
	fn on_bins_downloaded(
		&self,
		graph: &DependencyGraphWithTarget,
	) -> impl Future<Output = Result<(), Self::Error>> + Send {
		future::ready(Ok(()))
	}

	/// Called after all dependencies have been downloaded. The
	/// `downloaded_graph` contains all downloaded packages.
	fn on_all_downloaded(
		&self,
		graph: &DependencyGraphWithTarget,
	) -> impl Future<Output = Result<(), Self::Error>> + Send {
		future::ready(Ok(()))
	}
}

impl DownloadAndLinkHooks for () {
	type Error = Infallible;
}

/// Options for downloading and linking.
#[derive(Debug)]
pub struct DownloadAndLinkOptions<Reporter = (), Hooks = ()> {
	/// The reqwest client.
	pub reqwest: reqwest::Client,
	/// The downloads reporter.
	pub reporter: Option<Arc<Reporter>>,
	/// The download and link hooks.
	pub hooks: Option<Arc<Hooks>>,
	/// The refreshed sources.
	pub refreshed_sources: RefreshedSources,
	/// Whether to skip dev dependencies.
	pub prod: bool,
	/// The max number of concurrent network requests.
	pub network_concurrency: NonZeroUsize,
}

impl<Reporter, Hooks> DownloadAndLinkOptions<Reporter, Hooks>
where
	Reporter: for<'a> DownloadsReporter<'a> + Send + Sync + 'static,
	Hooks: DownloadAndLinkHooks + Send + Sync + 'static,
{
	/// Creates a new download options with the given reqwest client and reporter.
	pub fn new(reqwest: reqwest::Client) -> Self {
		Self {
			reqwest,
			reporter: None,
			hooks: None,
			refreshed_sources: Default::default(),
			prod: false,
			network_concurrency: NonZeroUsize::new(16).unwrap(),
		}
	}

	/// Sets the downloads reporter.
	pub fn reporter(mut self, reporter: impl Into<Arc<Reporter>>) -> Self {
		self.reporter.replace(reporter.into());
		self
	}

	/// Sets the download and link hooks.
	pub fn hooks(mut self, hooks: impl Into<Arc<Hooks>>) -> Self {
		self.hooks.replace(hooks.into());
		self
	}

	/// Sets the refreshed sources.
	pub fn refreshed_sources(mut self, refreshed_sources: RefreshedSources) -> Self {
		self.refreshed_sources = refreshed_sources;
		self
	}

	/// Sets whether to skip dev dependencies.
	pub fn prod(mut self, prod: bool) -> Self {
		self.prod = prod;
		self
	}

	/// Sets the max number of concurrent network requests.
	pub fn network_concurrency(mut self, network_concurrency: NonZeroUsize) -> Self {
		self.network_concurrency = network_concurrency;
		self
	}
}

impl Clone for DownloadAndLinkOptions {
	fn clone(&self) -> Self {
		Self {
			reqwest: self.reqwest.clone(),
			reporter: self.reporter.clone(),
			hooks: self.hooks.clone(),
			refreshed_sources: self.refreshed_sources.clone(),
			prod: self.prod,
			network_concurrency: self.network_concurrency,
		}
	}
}

impl Project {
	/// Downloads a graph of dependencies and links them in the correct order
	#[instrument(skip_all, fields(prod = options.prod), level = "debug")]
	pub async fn download_and_link<Reporter, Hooks>(
		&self,
		graph: &Arc<DependencyGraph>,
		options: DownloadAndLinkOptions<Reporter, Hooks>,
	) -> Result<DependencyGraphWithTarget, errors::DownloadAndLinkError<Hooks::Error>>
	where
		Reporter: for<'a> DownloadsReporter<'a> + 'static,
		Hooks: DownloadAndLinkHooks + 'static,
	{
		let DownloadAndLinkOptions {
			reqwest,
			reporter,
			hooks,
			refreshed_sources,
			prod,
			network_concurrency,
		} = options;

		let graph = graph.clone();
		let reqwest = reqwest.clone();
		let manifest = self.deser_manifest().await?;

		// step 1. download dependencies
		let downloaded_graph = {
			let mut downloaded_graph = BTreeMap::new();

			let mut download_graph_options = DownloadGraphOptions::<Reporter>::new(reqwest.clone())
				.refreshed_sources(refreshed_sources.clone())
				.network_concurrency(network_concurrency);

			if let Some(reporter) = reporter {
				download_graph_options = download_graph_options.reporter(reporter.clone());
			}

			let downloaded = self
				.download_graph(&graph, download_graph_options.clone())
				.instrument(tracing::debug_span!("download"))
				.await?;
			pin!(downloaded);

			let mut tasks = JoinSet::new();

			while let Some((id, node, fs)) = downloaded.try_next().await? {
				let container_folder =
					node.container_folder_from_project(&id, self, manifest.target.kind());

				if prod && node.resolved_ty == DependencyType::Dev {
					continue;
				}

				downloaded_graph.insert(id, node);

				let cas_dir = self.cas_dir().to_path_buf();
				tasks.spawn(async move {
					fs::create_dir_all(&container_folder).await?;
					fs.write_to(container_folder, cas_dir, true).await
				});
			}

			while let Some(task) = tasks.join_next().await {
				task.unwrap()?;
			}

			downloaded_graph
		};

		let (downloaded_wally_graph, downloaded_other_graph) = downloaded_graph
			.into_iter()
			.partition::<HashMap<_, _>, _>(|(_, node)| node.pkg_ref.is_wally_package());

		let mut graph = Arc::new(DependencyGraphWithTarget::new());

		async fn get_graph_targets<Hooks: DownloadAndLinkHooks>(
			graph: &mut Arc<DependencyGraphWithTarget>,
			project: &Project,
			manifest_target_kind: TargetKind,
			downloaded_graph: HashMap<PackageId, DependencyGraphNode>,
		) -> Result<(), errors::DownloadAndLinkError<Hooks::Error>> {
			let mut tasks = downloaded_graph
				.into_iter()
				.map(|(id, node)| {
					let source = node.pkg_ref.source();
					let path = Arc::from(
						node.container_folder_from_project(&id, project, manifest_target_kind)
							.as_path(),
					);
					let id = Arc::new(id.clone());
					let project = project.clone();

					async move {
						let target = source
							.get_target(
								&node.pkg_ref,
								&GetTargetOptions {
									project,
									path,
									id: id.clone(),
								},
							)
							.await?;

						Ok::<_, errors::DownloadAndLinkError<Hooks::Error>>((
							Arc::into_inner(id).unwrap(),
							DependencyGraphNodeWithTarget { node, target },
						))
					}
				})
				.collect::<JoinSet<_>>();

			while let Some(task) = tasks.join_next().await {
				let (id, node) = task.unwrap()?;
				Arc::get_mut(graph).unwrap().insert(id, node);
			}

			Ok(())
		}

		// step 2. get targets for non Wally packages (Wally packages require the scripts packages to be downloaded first)
		get_graph_targets::<Hooks>(
			&mut graph,
			self,
			manifest.target.kind(),
			downloaded_other_graph,
		)
		.instrument(tracing::debug_span!("get targets (non-wally)"))
		.await?;

		self.link_dependencies(graph.clone(), false)
			.instrument(tracing::debug_span!("link (non-wally)"))
			.await?;

		if let Some(hooks) = &hooks {
			hooks
				.on_scripts_downloaded(&graph)
				.await
				.map_err(errors::DownloadAndLinkError::Hook)?;

			hooks
				.on_bins_downloaded(&graph)
				.await
				.map_err(errors::DownloadAndLinkError::Hook)?;
		}

		// step 3. get targets for Wally packages
		get_graph_targets::<Hooks>(
			&mut graph,
			self,
			manifest.target.kind(),
			downloaded_wally_graph,
		)
		.instrument(tracing::debug_span!("get targets (wally)"))
		.await?;

		// step 4. link ALL dependencies. do so with types
		self.link_dependencies(graph.clone(), true)
			.instrument(tracing::debug_span!("link (all)"))
			.await?;

		if let Some(hooks) = &hooks {
			hooks
				.on_all_downloaded(&graph)
				.await
				.map_err(errors::DownloadAndLinkError::Hook)?;
		}

		Ok(Arc::into_inner(graph).unwrap())
	}
}

/// Errors that can occur when downloading and linking dependencies
pub mod errors {
	use thiserror::Error;

	/// An error that can occur when downloading and linking dependencies
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum DownloadAndLinkError<E> {
		/// Reading the manifest failed
		#[error("error reading manifest")]
		ManifestRead(#[from] crate::errors::ManifestReadError),

		/// An error occurred while downloading the graph
		#[error("error downloading graph")]
		DownloadGraph(#[from] crate::download::errors::DownloadGraphError),

		/// An error occurred while linking dependencies
		#[error("error linking dependencies")]
		Linking(#[from] crate::linking::errors::LinkingError),

		/// An error occurred while executing the pesde callback
		#[error("error executing hook")]
		Hook(#[source] E),

		/// IO error
		#[error("io error")]
		Io(#[from] std::io::Error),

		/// Error getting a target
		#[error("error getting target")]
		GetTarget(#[from] crate::source::errors::GetTargetError),
	}
}
