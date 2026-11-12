#![expect(deprecated)]
use std::collections::HashSet;
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context as _;
use futures::Stream;
use futures::TryStreamExt as _;
use pesde::Project;
use pesde::Url;
use pesde::WorkspaceMember;
use pesde::source::DependencySpecifier;
use pesde::source::InspectResult;
use pesde::source::PackageId;
use pesde::source::ResolveResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliDependencySpecifier {
	Full(DependencySpecifier),
	Named(Arc<str>, Box<str>),
}

impl FromStr for CliDependencySpecifier {
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if s.is_empty() {
			anyhow::bail!("specifier cannot be empty");
		}

		let parse_pesde = |url: Url, rest: &str| -> anyhow::Result<CliDependencySpecifier> {
			let (name, req) = rest
				.split_once('@')
				.context("invalid pesde specifier, missing @")?;

			Ok(Self::Full(DependencySpecifier::Pesde(
				pesde::source::PesdeDependencySpecifier {
					name: name.parse().context("invalid name")?,
					req: req.parse().context("invalid version requirement")?,
					registry: url,
				},
			)))
		};

		if let Some((url, rest)) = s.rsplit_once(':')
			&& let Ok(url) = Url::from_str(url)
            // `pesde:https://` will parse with a scheme of `pesde`.
            // we can hardcode these for convenience and expect explicit discrimination to be used for other schemes.
			&& matches!(url.scheme(), "http" | "https")
		{
			return parse_pesde(url, rest);
		}

		let Some((maybe_source, rest)) = s.split_once(':') else {
			return Ok(Self::Named("default".into(), s.into()));
		};

		match maybe_source {
			"pesde"
				if let Some((url, rest)) = rest
					.rsplit_once(':')
					.and_then(|(url, rest)| Url::from_str(url).ok().map(|url| (url, rest))) =>
			{
				return parse_pesde(url, rest);
			}
			"wally"
				if let Some((url, rest)) = rest
					.rsplit_once(':')
					.and_then(|(url, rest)| Url::from_str(url).ok().map(|url| (url, rest))) =>
			{
				let (name, req) = rest
					.split_once('@')
					.context("invalid wally specifier, missing @")?;

				return Ok(Self::Full(DependencySpecifier::Wally(
					pesde::source::WallyDependencySpecifier {
						name: name.parse().context("invalid name")?,
						req: req.parse().context("invalid version requirement")?,
						index: url,
					},
				)));
			}
			"legacy_pesde"
				if let Some((url, rest)) = rest
					.rsplit_once(':')
					.and_then(|(url, rest)| Url::from_str(url).ok().map(|url| (url, rest))) =>
			{
				let (name, rest) = rest
					.split_once('@')
					.context("invalid legacy pesde specifier, missing @")?;

				let (req, target) = rest
					.split_once(' ')
					.context("invalid legacy pesde specifier, missing target")?;

				return Ok(Self::Full(DependencySpecifier::LegacyPesde(
					pesde::source::LegacyPesdeDependencySpecifier {
						name: name.parse().context("invalid name")?,
						req: req.parse().context("invalid version requirement")?,
						target: target
							.parse()
							.map_err(|e| anyhow::format_err!("invalid target: {e}"))?,
						index: url,
					},
				)));
			}
			"git" if let Some((url, rest)) = rest.rsplit_once('#') => {
				let url = Url::from_str(url).context("invalid git url")?;

				let (rev, path) = rest.split_once(':').unwrap_or((rest, ""));

				return Ok(Self::Full(DependencySpecifier::Git(
					pesde::source::GitDependencySpecifier {
						url,
						rev: rev.into(),
						path: path.parse().context("invalid git path")?,
					},
				)));
			}
			_ => {}
		}

		Ok(Self::Named(maybe_source.into(), rest.into()))
	}
}

impl CliDependencySpecifier {
	pub async fn resolve(self, member: &WorkspaceMember) -> anyhow::Result<DependencySpecifier> {
		match self {
			CliDependencySpecifier::Full(specifier) => Ok(specifier),
			CliDependencySpecifier::Named(source, specifier) => {
				let manifest = member
					.deser_manifest()
					.await
					.context("failed to read manifest")?;

				let Some(specifier) = manifest.parse_specifier(&source, &specifier)? else {
					return Err(anyhow::format_err!(
						"unknown source `{source}`, perhaps the URL was invalid?"
					));
				};

				Ok(specifier)
			}
		}
	}
}

pub async fn resolve(
	project: &Project,
	specifier: DependencySpecifier,
) -> anyhow::Result<impl Stream<Item = anyhow::Result<(PackageId, Option<InspectResult>)>> + use<>>
{
	let source_provider = project.source_provider();

	let mut visited = HashSet::new();
	let mut queue = VecDeque::from([specifier.source_id()]);

	while let Some(source) = queue.pop_front() {
		if !visited.insert(source.clone()) {
			anyhow::bail!("circular source dependency detected while resolving specifier");
		}

		let source = source_provider
			.get(&source)
			.await
			.map_err(anyhow::Error::from_boxed)
			.context("failed to get package source")?;

		let result = source
			.resolve(&specifier)
			.await
			.map_err(anyhow::Error::from_boxed)
			.context("failed to resolve package")?;

		match result {
			ResolveResult::Found(stream) => return Ok(stream.map_err(anyhow::Error::from_boxed)),
			ResolveResult::Elsewhere(sources) => {
				queue.extend(sources);
			}
		}
	}

	anyhow::bail!("something went terribly wrong, resolution did not complete with a source");
}

#[cfg(test)]
mod tests {
	#![expect(deprecated)]
	use pesde::source::DependencySpecifier;
	use pesde::source::LegacyPesdeDependencySpecifier;
	use pesde::source::PesdeDependencySpecifier;
	use pesde::source::WallyDependencySpecifier;
	use pesde::vfs::VfsPath;

	#[test]
	fn test_cli_dependency_specifier_from_str() {
		use super::CliDependencySpecifier;
		use std::str::FromStr as _;

		let pesde_specifier =
			CliDependencySpecifier::Full(DependencySpecifier::Pesde(PesdeDependencySpecifier {
				name: "hello/world".parse().unwrap(),
				req: "1.0.0".parse().unwrap(),
				registry: "https://registry.pesde.dev".parse().unwrap(),
			}));

		let expected = [
			(
				"hello/world",
				CliDependencySpecifier::Named("default".into(), "hello/world".into()),
			),
			(
				"default:hello/world",
				CliDependencySpecifier::Named("default".into(), "hello/world".into()),
			),
			(
				"https://registry.pesde.dev:hello/world@1.0.0",
				pesde_specifier.clone(),
			),
			(
				"pesde:https://registry.pesde.dev:hello/world@1.0.0",
				pesde_specifier,
			),
			(
				"pesde:hello/world@1.0.0",
				CliDependencySpecifier::Named("pesde".into(), "hello/world@1.0.0".into()),
			),
			(
				"wally:https://github.com/UpliftGames/wally-index:hello/world@1.0.0",
				CliDependencySpecifier::Full(DependencySpecifier::Wally(
					WallyDependencySpecifier {
						name: "hello/world".parse().unwrap(),
						req: "1.0.0".parse().unwrap(),
						index: "https://github.com/UpliftGames/wally-index"
							.parse()
							.unwrap(),
					},
				)),
			),
			(
				"legacy_pesde:https://registry.pesde.dev:hello/world@1.0.0 roblox",
				CliDependencySpecifier::Full(DependencySpecifier::LegacyPesde(
					LegacyPesdeDependencySpecifier {
						name: "hello/world".parse().unwrap(),
						req: "1.0.0".parse().unwrap(),
						target: "roblox".parse().unwrap(),
						index: "https://registry.pesde.dev".parse().unwrap(),
					},
				)),
			),
			(
				"git:https://github.com/user/repo.git#main",
				CliDependencySpecifier::Full(DependencySpecifier::Git(
					pesde::source::GitDependencySpecifier {
						url: "https://github.com/user/repo.git".parse().unwrap(),
						rev: "main".into(),
						path: VfsPath::root(),
					},
				)),
			),
			(
				"git:https://github.com/user/repo.git#main:path/to/dir",
				CliDependencySpecifier::Full(DependencySpecifier::Git(
					pesde::source::GitDependencySpecifier {
						url: "https://github.com/user/repo.git".parse().unwrap(),
						rev: "main".into(),
						path: "path/to/dir".parse().unwrap(),
					},
				)),
			),
		];

		for (input, expected) in expected {
			let result = CliDependencySpecifier::from_str(input)
				.unwrap_or_else(|_| panic!("failed to parse input: {input}"));
			assert_eq!(result, expected, "failed for input: {input}");
		}
	}
}
