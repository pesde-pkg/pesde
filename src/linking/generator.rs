use std::{
	borrow::Cow,
	path::{Component, Path, PathBuf},
};

use crate::{
	manifest::{Manifest, target::TargetKind},
	source::refs::StructureKind,
};
use full_moon::{ast::luau::ExportedTypeDeclaration, visitors::Visitor};
use relative_path::RelativePath;
use tracing::instrument;

/// Paths used for linking
#[derive(Debug)]
pub struct LinkDirs {
	/// the root directory of the packages, e.g. `/path/to/project/luau_packages` or `/path/to/project/luau_packages/.pesde/my+package/package/luau_packages`
	pub base: PathBuf,
	/// the directory in which the library is contained, e.g. `/path/to/project/luau_packages/.pesde/my+package/package`
	pub destination: PathBuf,
	/// the root directory of the packages, e.g. `/path/to/project/luau_packages`
	pub root_container: PathBuf,
	/// Relative path used when linking Roblox places - appended to the place path. For example, `.pesde/my+package/package`
	pub container: PathBuf,
}

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

fn luau_style_path(path: &Path) -> String {
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
			Component::Normal(part) if part != "init.lua" && part != "init.luau" => {
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

	let require = format!("./{path}");
	format!("{require:?}")
}

// This function should be simplified (especially to reduce the number of arguments),
// but it's not clear how to do that while maintaining the current functionality.
/// Get the require path for a library
#[instrument(skip(project_manifest), level = "trace", ret)]
pub fn get_lib_require_path(
	target: TargetKind,
	lib_file: &RelativePath,
	dirs: &LinkDirs,
	structure_kind: StructureKind,
	project_manifest: &Manifest,
) -> Result<String, errors::GetLibRequirePath> {
	let path = pathdiff::diff_paths(&dirs.destination, &dirs.base).unwrap();
	tracing::debug!("diffed lib path: {}", path.display());
	let path = match structure_kind {
		StructureKind::Wally => path,
		StructureKind::PesdeV1 => lib_file.to_path(path),
	};

	let (prefix, path) = match target.try_into() {
		Ok(place_kind) if !dirs.destination.starts_with(&dirs.root_container) => (
			project_manifest
				.place
				.get(&place_kind)
				.ok_or(errors::GetLibRequirePath::RobloxPlaceKindPathNotFound(
					place_kind,
				))?
				.as_str(),
			match structure_kind {
				StructureKind::Wally => Cow::Borrowed(&dirs.container),
				StructureKind::PesdeV1 => Cow::Owned(lib_file.to_path(&dirs.container)),
			},
		),
		Ok(_) if structure_kind == StructureKind::Wally => ("script.Parent", Cow::Owned(path)),
		_ => return Ok(luau_style_path(&path)),
	};

	let path = path
		.components()
		.zip(
			path.components()
				.skip(1)
				.map(Some)
				.chain(std::iter::repeat(None)),
		)
		.filter_map(|(component, next_comp)| match component {
			Component::ParentDir => Some(".Parent".to_string()),
			Component::Normal(part) if part != "init.lua" && part != "init.luau" => {
				let str = part.to_string_lossy();

				Some(format!(
					":FindFirstChild({:?})",
					if next_comp.is_some() {
						&str
					} else {
						str.strip_suffix(".luau")
							.or_else(|| str.strip_suffix(".lua"))
							.unwrap_or(&str)
					}
				))
			}
			_ => None,
		})
		.collect::<String>();

	Ok(format!("{prefix}{path}"))
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

	luau_style_path(&path)
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
