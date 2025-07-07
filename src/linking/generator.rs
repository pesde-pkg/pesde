use std::path::{Component, Path, PathBuf};

use crate::manifest::{target::TargetKind, Manifest};
use full_moon::{ast::luau::ExportedTypeDeclaration, visitors::Visitor};
use relative_path::RelativePath;
use tracing::instrument;

struct TypeVisitor {
	types: Vec<String>,
}

impl Visitor for TypeVisitor {
	fn visit_exported_type_declaration(&mut self, node: &ExportedTypeDeclaration) {
		let name = node.type_declaration().type_name().to_string();

		let (declaration_generics, generics) =
			if let Some(declaration) = node.type_declaration().generics() {
				let mut declaration_generics = vec![];
				let mut generics = vec![];

				for generic in declaration.generics() {
					declaration_generics.push(generic.to_string());

					if generic.default_type().is_some() {
						generics.push(generic.parameter().to_string());
					} else {
						generics.push(generic.to_string());
					}
				}

				(
					format!("<{}>", declaration_generics.join(", ")),
					format!("<{}>", generics.join(", ")),
				)
			} else {
				("".to_string(), "".to_string())
			};

		self.types.push(format!(
			"export type {name}{declaration_generics} = module.{name}{generics}\n"
		));
	}
}

pub(crate) fn get_file_types(file: &str) -> Vec<String> {
	let ast = match full_moon::parse(file) {
		Ok(ast) => ast,
		Err(err) => {
			tracing::error!(
				"failed to parse file to extract types:\n{}",
				err.into_iter()
					.map(|err| format!("\t- {err}"))
					.collect::<Vec<_>>()
					.join("\n")
			);

			return vec![];
		}
	};
	let mut visitor = TypeVisitor { types: vec![] };
	visitor.visit_ast(&ast);

	visitor.types
}

/// Generate a linking module for a library
#[must_use]
pub fn generate_lib_linking_module<I: IntoIterator<Item = S>, S: AsRef<str>>(
	path: &str,
	types: I,
) -> String {
	let mut output = format!("local module = require({path})\n");

	for ty in types {
		output.push_str(ty.as_ref());
	}

	output.push_str("return module");

	output
}

fn luau_style_path(path: &Path, leading_slash: bool) -> String {
	let path = path
		.components()
		.zip(
			path.components()
				.skip(1)
				.map(Some)
				.chain(std::iter::repeat(None)),
		)
		.filter_map(|(ct, next_ct)| match ct {
			Component::CurDir => Some(".".to_string()),
			Component::ParentDir => Some("..".to_string()),
			Component::Normal(part) => {
				let str = part.to_string_lossy();

				Some(
					(if next_ct.is_some() {
						&str
					} else {
						str.strip_suffix(".luau")
							.or_else(|| str.strip_suffix(".lua"))
							.unwrap_or(&str)
					})
					.to_string(),
				)
			}
			_ => None,
		})
		.collect::<Vec<_>>()
		.join("/");

	let prefix = if leading_slash { "./" } else { "" };
	let require = format!("{prefix}{path}");
	format!("{require:?}")
}

// This function should be simplified (especially to reduce the number of arguments),
// but it's not clear how to do that while maintaining the current functionality.
/// Get the require path for a library
#[instrument(skip(project_manifest), level = "trace", ret)]
#[allow(clippy::too_many_arguments)]
pub fn get_lib_require_path(
	target: TargetKind,
	base_dir: &Path,
	lib_file: &RelativePath,
	destination_dir: &Path,
	use_new_structure: bool,
	root_container_dir: &Path,
	container_dir: &Path,
	project_manifest: &Manifest,
) -> Result<String, errors::GetLibRequirePath> {
	let path = pathdiff::diff_paths(destination_dir, base_dir).unwrap();
	tracing::debug!("diffed lib path: {}", path.display());
	let path = if use_new_structure {
		lib_file.to_path(path)
	} else {
		path
	};

	let (leading_slash, prefix, path) = match (target, target.try_into()) {
		(TargetKind::Roblox | TargetKind::RobloxServer, Ok(place_kind))
			if !destination_dir.starts_with(root_container_dir) =>
		{
			(
				false,
				PathBuf::from(
					project_manifest
						.place
						.get(&place_kind)
						.ok_or(errors::GetLibRequirePath::RobloxPlaceKindPathNotFound(
							place_kind,
						))?
						.replace('.', "/")
						.replace(['[', ']', '\'', '"'], ""),
				),
				if use_new_structure {
					lib_file.to_path(container_dir)
				} else {
					container_dir.to_path_buf()
				}
			)
		}

		_ => (true, PathBuf::new(), path),
	};
	let path = prefix.join(path);

	Ok(luau_style_path(&path, leading_slash))
}

/// Generate a linking module for a binary
#[must_use]
pub fn generate_bin_linking_module<P: AsRef<Path>>(package_root: P, require_path: &str) -> String {
	format!(
		r"_G.PESDE_ROOT = {:?}
return require({require_path})",
		package_root.as_ref().to_string_lossy()
	)
}

/// Get the require path for a binary
#[instrument(level = "trace", ret)]
#[must_use]
pub fn get_bin_require_path(
	base_dir: &Path,
	bin_file: &RelativePath,
	destination_dir: &Path,
) -> String {
	let path = pathdiff::diff_paths(destination_dir, base_dir).unwrap();
	tracing::debug!("diffed bin path: {}", path.display());
	let path = bin_file.to_path(path);

	luau_style_path(&path, true)
}

/// Generate a linking module for a script
#[must_use]
pub fn generate_script_linking_module(require_path: &str) -> String {
	format!(r"return require({require_path})")
}

/// Get the require path for a script
#[instrument(level = "trace", ret)]
#[must_use]
pub fn get_script_require_path(
	base_dir: &Path,
	script_file: &RelativePath,
	destination_dir: &Path,
) -> String {
	let path = pathdiff::diff_paths(destination_dir, base_dir).unwrap();
	tracing::debug!("diffed script path: {}", path.display());
	let path = script_file.to_path(path);

	luau_style_path(&path, true)
}

/// Errors for the linking module utilities
pub mod errors {
	use thiserror::Error;

	/// An error occurred while getting the require path for a library
	#[derive(Debug, Error)]
	#[non_exhaustive]
	pub enum GetLibRequirePath {
		/// The path for the RobloxPlaceKind could not be found
		#[error("could not find the path for the RobloxPlaceKind {0}")]
		RobloxPlaceKindPathNotFound(crate::manifest::target::RobloxPlaceKind),
	}
}
