use async_trait::async_trait;
use croshet::ExecuteResult;
use croshet::ShellCommand;
use croshet::ShellPipeReader;
use croshet::ShellPipeWriter;
use itertools::Either;
use std::collections::HashMap;
use std::convert::Infallible;
use std::error::Error;
use std::future;
use std::sync::Arc;
use tracing::instrument;

use crate::PACKAGES_CONTAINER_NAME;
use crate::Subproject;
use crate::resolver::DependencyGraph;
use crate::resolver::DependencyGraphNode;
use crate::source::RealmExt as _;
use crate::source::ids::PackageId;
use crate::source::traits::PackageExports;

/// Hooks for [execute_script]
#[allow(unused_variables)]
pub trait ExecuteScriptHooks {
	/// The error the methods return
	type Error: Error;

	/// Returns the stdio options in the format of (stdout, stderr, stdin)
	fn stdio(&mut self) -> (ShellPipeWriter, ShellPipeWriter, ShellPipeReader) {
		(
			ShellPipeWriter::Stdout,
			ShellPipeWriter::Stderr,
			ShellPipeReader::stdin(),
		)
	}

	/// Called when the script is being executed
	fn run(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
		future::ready(Ok(()))
	}
}

impl ExecuteScriptHooks for () {
	type Error = Infallible;
}

struct BinPackageCommand {
	context: ExecuteScriptContext,
}

#[async_trait]
impl ShellCommand for BinPackageCommand {
	async fn execute(&self, context: croshet::ShellCommandContext) -> ExecuteResult {
		struct Hooks {
			stdin: Option<ShellPipeReader>,
			stdout: Option<ShellPipeWriter>,
			stderr: Option<ShellPipeWriter>,
		}

		impl ExecuteScriptHooks for Hooks {
			type Error = Infallible;

			fn stdio(&mut self) -> (ShellPipeWriter, ShellPipeWriter, ShellPipeReader) {
				(
					self.stdout.take().unwrap(),
					self.stderr.take().unwrap(),
					self.stdin.take().unwrap(),
				)
			}
		}

		let package = self.context.package.as_ref().unwrap();
		execute_script(
			self.context.package_exports[package]
				.x_script
				.as_deref()
				.unwrap(),
			self.context.clone(),
			&mut Hooks {
				stdin: Some(context.stdin),
				stdout: Some(context.stdout),
				stderr: Some(context.stderr),
			},
			context.args,
		)
		.await;

		ExecuteResult::Exit(0, vec![])
	}
}

#[derive(Debug, Clone)]
pub struct ExecuteScriptContext {
	graph: Arc<DependencyGraph>,
	subproject: Subproject,
	package_exports: Arc<HashMap<PackageId, Arc<PackageExports>>>,
	package: Option<PackageId>,
}

impl ExecuteScriptContext {
	pub fn new(
		graph: impl Into<Arc<DependencyGraph>>,
		subproject: Subproject,
		package_exports: impl Into<Arc<HashMap<PackageId, Arc<PackageExports>>>>,
	) -> Self {
		Self {
			graph: graph.into(),
			subproject,
			package_exports: package_exports.into(),
			package: None,
		}
	}
}

/// Executes a script
#[instrument(skip(hooks), level = "debug")]
pub async fn execute_script<H: ExecuteScriptHooks>(
	script: &str,
	context: ExecuteScriptContext,
	hooks: &mut H,
	args: Vec<std::ffi::OsString>,
) -> Result<i32, errors::ExecuteScriptError<H>> {
	let parsed_script = croshet::parser::parse(script)?;

	let (stdout, stderr, stdin) = hooks.stdio();

	let commands = match &context.package {
		Some(package) => Either::Left(
			context.graph.nodes[package]
				.dependencies
				.iter()
				.map(|(alias, (id, _, _))| (alias, id)),
		),
		None => Either::Right(
			context.graph.importers[context.subproject.importer()]
				.dependencies
				.iter()
				.map(|(alias, (id, _, _))| (alias, id)),
		),
	}
	.filter(|(_, id)| {
		context
			.package_exports
			.get(id)
			.is_some_and(|exports| exports.x_script.is_some())
	})
	.map(|(alias, id)| {
		(
			alias.as_str().to_string(),
			Arc::new(BinPackageCommand {
				context: ExecuteScriptContext {
					package: Some(id.clone()),
					..context.clone()
				},
			}) as Arc<dyn ShellCommand>,
		)
	})
	.collect();

	let (code, stdio_result) = tokio::join!(
		croshet::execute(
			parsed_script,
			croshet::ExecuteOptionsBuilder::new()
				.cwd(context.subproject.dir().to_path_buf())
				.stdout(stdout)
				.stderr(stderr)
				.stdin(stdin)
				.args(args)
				.custom_commands(commands)
				.env_var(
					"PESDE_ROOT".into(),
					match &context.package {
						Some(package) => context
							.subproject
							.dependencies_dir()
							.join(
								context
									.graph
									.realm_of(context.subproject.importer(), package)
									.packages_dir(),
							)
							.join(PACKAGES_CONTAINER_NAME)
							.join(DependencyGraphNode::container_dir(package))
							.into_os_string(),
						None => context.subproject.dir().into_os_string(),
					}
				)
				.build()
				.unwrap(),
		),
		hooks.run(),
	);
	stdio_result.map_err(errors::ExecuteScriptError::Hooks)?;

	Ok(code)
}

/// Errors that can occur when using scripts
pub mod errors {
	use thiserror::Error;

	use crate::scripts::ExecuteScriptHooks;

	/// Errors which can occur while executing a script
	#[derive(Debug, Error)]
	pub enum ExecuteScriptError<Hooks: ExecuteScriptHooks> {
		/// Constructing a PATH failed
		#[error("PATH creation error")]
		Path(#[from] std::env::JoinPathsError),

		/// Script parsing failed
		#[error("script parsing failed")]
		Parse(#[from] croshet::Error),

		/// The hooks have errored
		#[error("error executing hook")]
		Hooks(Hooks::Error),
	}
}
