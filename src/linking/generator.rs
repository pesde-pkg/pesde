use std::borrow::Cow;
use std::fmt::Display;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use crate::manifest::Manifest;
use crate::manifest::target::TargetKind;
use crate::source::refs::StructureKind;
use full_moon::ast::luau::ExportedTypeDeclaration;
use full_moon::visitors::Visitor;
use itertools::Itertools as _;
use itertools::Position;
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
		let name = node.type_declaration().type_name();

		let mut declaration_generics: Vec<&dyn Display> = vec![];
		let mut generics: Vec<&dyn Display> = vec![];

		if let Some(declaration) = node.type_declaration().generics() {
			for generic in declaration.generics() {
				declaration_generics.push(generic);

				if generic.default_type().is_some() {
					generics.push(generic.parameter());
				} else {
					generics.push(generic);
				}
			}
		}

		let declaration_generics = if declaration_generics.is_empty() {
			format_args!("")
		} else {
			format_args!("<{}>", declaration_generics.into_iter().format(", "))
		};
		let generics = if generics.is_empty() {
			format_args!("")
		} else {
			format_args!("<{}>", generics.into_iter().format(", "))
		};

		self.types.push(format!(
			"export type {name}{declaration_generics} = module.{name}{generics}"
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
					.format_with("\n", |err, f| f(&format_args!("\t- {err}")))
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
	let types = types.into_iter().format_with("\n", |ty, f| f(&ty.as_ref()));

	format!("local module = require({path})\n{types}\nreturn module")
}

fn luau_style_path(path: &Path) -> String {
	let path = path
		.components()
		.with_position()
		.filter_map(|(pos, ct)| match ct {
			Component::CurDir => Some(".".into()),
			Component::ParentDir => Some("..".into()),
			Component::Normal(part) if part != "init.lua" && part != "init.luau" => {
				let str = part.to_string_lossy();

				Some(
					if matches!(pos, Position::Last | Position::Only)
						&& let Some(str) = str.strip_suffix(".luau").or(str.strip_suffix(".lua"))
					{
						Cow::Owned(str.to_string())
					} else {
						str
					},
				)
			}
			_ => None,
		})
		.format("/");

	let require = format!("./{path}");
	format!(r#""{}""#, require.escape_default())
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
				.ok_or(errors::GetLibRequirePathKind::RobloxPlaceKindPathNotFound(
					place_kind,
				))?
				.as_str(),
			match structure_kind {
				StructureKind::Wally => Cow::Borrowed(dirs.container.as_path()),
				StructureKind::PesdeV1 => Cow::Owned(lib_file.to_path(&dirs.container)),
			},
		),
		Ok(_) if structure_kind == StructureKind::Wally => ("script.Parent", Cow::Owned(path)),
		_ => return Ok(luau_style_path(&path)),
	};

	let path = path
		.components()
		.with_position()
		.filter_map(|(pos, component)| match component {
			Component::ParentDir => Some(Cow::Borrowed(".Parent")),
			Component::Normal(part) if part != "init.lua" && part != "init.luau" => {
				let str = part.to_string_lossy();

				Some(
					format!(
						r#":FindFirstChild("{}")"#,
						if matches!(pos, Position::Last | Position::Only) {
							str.strip_suffix(".luau")
								.or(str.strip_suffix(".lua"))
								.unwrap_or(&str)
						} else {
							&str
						}
						.escape_debug()
					)
					.into(),
				)
			}
			_ => None,
		})
		.format("");

	Ok(format!("{prefix}{path}"))
}

/// Generate a linking module for a binary
#[must_use]
pub fn generate_bin_linking_module(package_root: &Path, require_path: &str) -> String {
	format!(
		r#"_G.PESDE_ROOT = "{}"
return require({require_path})"#,
		package_root.to_string_lossy().escape_default()
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
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GetLibRequirePath))]
	#[non_exhaustive]
	pub enum GetLibRequirePathKind {
		/// The path for the RobloxPlaceKind could not be found
		#[error("could not find the path for the RobloxPlaceKind {0}")]
		RobloxPlaceKindPathNotFound(crate::manifest::target::RobloxPlaceKind),
	}
}
