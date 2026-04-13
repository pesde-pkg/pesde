//! Generates linking modules for a project
use std::borrow::Cow;
use std::fmt::Display;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use crate::manifest::Manifest;
use crate::source::Realm;
use crate::source::StructureKind;
use full_moon::ast::luau::ExportedTypeDeclaration;
use full_moon::ast::luau::ExportedTypeFunction;
use full_moon::visitors::Visitor;
use itertools::Itertools as _;
use itertools::Position;
use relative_path::RelativePath;
use tracing::instrument;

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

	fn visit_exported_type_function(&mut self, node: &ExportedTypeFunction) {
		let name = node.type_function().function_name();
		let params = node.type_function().function_body().parameters();

		// Not possible to re-export type functions without parameters as a type declaration
		if params.is_empty() {
			return;
		}

		let declaration_generics = format_args!("<{}>", params.iter().format(", "));
		let generics = format_args!("<{}>", params.iter().format(", "));

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

fn luau_style_path(path: &Path) -> impl Display {
	path.components()
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
		.format("/")
}

fn relative_luau_path(path: &Path) -> String {
	format!(
		r#""{}""#,
		format!("./{}", luau_style_path(path)).escape_default()
	)
}

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

/// Get the require path for a library
#[instrument(skip(project_manifest), level = "trace", ret)]
pub fn get_lib_require_path(
	realm: Option<Realm>,
	lib_file: &RelativePath,
	dirs: &LinkDirs,
	structure_kind: &StructureKind,
	project_manifest: &Manifest,
) -> Result<String, errors::GetLibRequirePath> {
	let path = pathdiff::diff_paths(&dirs.destination, &dirs.base).unwrap();
	tracing::debug!("diffed lib path: {}", path.display());
	let path = match structure_kind {
		StructureKind::Wally(_) => path,
		StructureKind::PesdeV1(_) | StructureKind::PesdeV2 => lib_file.to_path(path),
	};

	let Some(realm) = realm.filter(|_| !dirs.destination.starts_with(&dirs.root_container)) else {
		return Ok(relative_luau_path(&path));
	};

	let Some(absolute_prefix) = project_manifest.absolute_paths.get(&realm) else {
		return Err(errors::GetLibRequirePathKind::RealmPathNotFound(realm).into());
	};
	let path = match structure_kind {
		StructureKind::Wally(_) => Cow::Borrowed(dirs.container.as_path()),
		StructureKind::PesdeV1(_) | StructureKind::PesdeV2 => {
			Cow::Owned(lib_file.to_path(&dirs.container))
		}
	};

	Ok(format!(
		r#""{}""#,
		format!(
			"{}/{}",
			absolute_prefix.strip_suffix('/').unwrap_or(absolute_prefix),
			luau_style_path(&path)
		)
		.escape_default()
	))
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

	relative_luau_path(&path)
}

/// Errors for the linking module utilities
pub mod errors {
	use thiserror::Error;

	/// An error occurred while getting the require path for a library
	#[derive(Debug, Error, thiserror_ext::Box)]
	#[thiserror_ext(newtype(name = GetLibRequirePath))]
	#[non_exhaustive]
	pub enum GetLibRequirePathKind {
		/// The path for the realm could not be found
		#[error("could not find the path for the {0} realm")]
		RealmPathNotFound(crate::source::Realm),
	}
}
