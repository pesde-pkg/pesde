#![expect(deprecated)]
use std::collections::HashSet;

use crate::registry::manifest::Alias;
use crate::registry::manifest::ManifestExports;
use crate::registry::manifest::config_luau::LuauConfig;
use crate::registry::manifest::wally_compat::WallyLocalName;
use futures::StreamExt as _;
use serde::Serialize as _;

use crate::__private::str_as_fs_safe;
use crate::Error;
use crate::Result;
use crate::WorkspaceMember;
use crate::legacy::TargetKind;
use crate::source::PackageId;
use crate::vfs::PackageStorer;
use crate::vfs::VfsPath;

// A light wrapper over Vfs methods that is context-specific to linking.
pub(crate) struct LinkVfs {
	pub(crate) member: WorkspaceMember,
	dependencies_dir: VfsPath,
}

impl LinkVfs {
	pub(crate) fn new(member: WorkspaceMember) -> Self {
		Self {
			dependencies_dir: member
				.importer()
				.join(&"pesde/dependencies".parse().unwrap()),
			member,
		}
	}

	pub(crate) async fn clean_dependencies_dir(&self) -> Result<()> {
		match self
			.member
			.workspace()
			.vfs()
			.remove_dir_all(&self.dependencies_dir)
			.await
		{
			Ok(()) => Ok(()),
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
			Err(e) => Err(Error::vfs(e, self.dependencies_dir.clone())),
		}
	}

	fn packages_dir(&self) -> VfsPath {
		self.dependencies_dir.join(&".pesde".parse().unwrap())
	}

	fn package_dir_name(package: &PackageId) -> VfsPath {
		str_as_fs_safe(&package.to_string()).parse().unwrap()
	}

	fn package_dir(&self, package: &PackageId) -> VfsPath {
		self.packages_dir().join(&Self::package_dir_name(package))
	}

	pub(crate) async fn package_dir_exists(&self, package: &PackageId) -> Result<bool> {
		let path = self.package_dir(package);
		self.member
			.workspace()
			.vfs()
			.exists(&path)
			.await
			.map_err(|e| Error::vfs(e, path))
	}

	fn install_directory(&self, package: &PackageId, structure: &PackageStructure) -> VfsPath {
		let mut path = self.package_dir(package);
		match structure {
			PackageStructure::Pesde(..) => {}
			PackageStructure::Wally(name) => path = path.join(&name.parse().unwrap()),
			PackageStructure::LegacyPesde(..) => {}
		}
		path
	}

	pub(crate) async fn install_package(
		&self,
		package: &PackageId,
		structure: &PackageStructure,
		will_download: bool,
	) -> Result<Option<Box<dyn PackageStorer>>> {
		let install_dir = self.install_directory(package, structure);
		let vfs = self.member.workspace().vfs();

		let result = if will_download {
			vfs.copy_and_maybe_store_package(true, package, structure.kind(), &install_dir)
				.await
		} else {
			vfs.copy_package_to(true, package, structure.kind(), &install_dir)
				.await
				.map(|_| None)
		};

		result.map_err(|e| Error::vfs(e, install_dir))
	}

	fn require_path_for(&self, package: &PackageId, structure: &PackageStructure) -> VfsPath {
		let dir = self.install_directory(package, structure);
		match structure {
			PackageStructure::Pesde(exports, _) | PackageStructure::LegacyPesde(exports, _) => {
				// TODO: handle libless (binary only)
				dir.join(&exports.lib().unwrap().parse().unwrap())
			}
			PackageStructure::Wally(_) => dir,
		}
	}

	async fn write_config_luau(
		&self,
		context: std::fmt::Arguments<'_>,
		mut config: LuauConfig,
		in_path: &VfsPath,
		dependencies: impl Iterator<Item = (&Alias, VfsPath)>,
	) -> Result<()> {
		let aliases = config.aliases_mut();

		for (alias, dependency_dir) in dependencies {
			let relative = in_path.diff(&dependency_dir);
			match aliases.entry(alias.clone()) {
				std::collections::btree_map::Entry::Occupied(entry) => {
					tracing::warn!("duplicate alias `{}` for {context}", entry.key());
				}
				std::collections::btree_map::Entry::Vacant(entry) => {
					entry.insert(relative);
				}
			}
		}

		let mut serializer = LuauSerializer {
			buf: b"return ".to_vec(),
			level: 0,
		};
		config
			.serialize(&mut serializer)
			.expect("LuauConfig should always serialize successfully");
		let contents = serializer.buf;

		let file_path = in_path.join(&".config.luau".parse().unwrap());

		self.member
			.workspace()
			.vfs()
			.write_file(&file_path, &contents)
			.await
			.map_err(|e| Error::vfs(e, file_path))?;

		Ok(())
	}

	pub(crate) async fn link_package_depenencies(
		&self,
		for_package: PackageLinkContext<'_>,
		dependencies: impl Iterator<Item = (&Alias, PackageLinkContext<'_>)>,
	) -> Result<()> {
		let for_package_dir = self.install_directory(for_package.package_id, for_package.structure);

		match &for_package.structure {
			PackageStructure::Pesde(_, config) => {
				self.write_config_luau(
					format_args!("package {}", for_package.package_id),
					config.clone(),
					&for_package_dir,
					dependencies.map(|(alias, dependency)| {
						(
							alias,
							self.require_path_for(dependency.package_id, dependency.structure),
						)
					}),
				)
				.await?;
			}
			PackageStructure::Wally(_) => {
				let dependencies_dir = for_package_dir.pop().unwrap().0;
				for (alias, dependency) in dependencies {
					let context = if for_package.context.is_empty() {
						format_args!("")
					} else {
						format_args!("{}\n", for_package.context)
					};
					let contents = format!(
						"local module = require(\"{}\")\n{context}return module",
						dependencies_dir
							.diff(
								&self.require_path_for(dependency.package_id, dependency.structure)
							)
							.escape_default(),
					);

					let path = dependencies_dir.join(&format!("{alias}.luau").parse().unwrap());

					self.member
						.workspace()
						.vfs()
						.write_file(&path, contents.as_bytes())
						.await
						.map_err(|e| Error::vfs(e, path))?;
				}
			}
			PackageStructure::LegacyPesde(_, _) => {
				for (alias, dependency) in dependencies {
					let dependencies_dir = match &dependency.structure {
						PackageStructure::Pesde(..) => {
							tracing::error!(
								"impossible: legacy pesde package depends on a pesde package"
							);
							continue;
						}
						#[expect(clippy::todo)]
						PackageStructure::Wally(_) => {
							// TODO: this needs to get the realm of the Wally package
							todo!()
						}
						PackageStructure::LegacyPesde(_, target) => {
							for_package_dir.join(&target.packages_dir().parse().unwrap())
						}
					};

					let context = if for_package.context.is_empty() {
						format_args!("")
					} else {
						format_args!("{}\n", for_package.context)
					};
					let contents = format!(
						"local module = require(\"{}\")\n{context}return module",
						dependencies_dir
							.diff(
								&self.require_path_for(dependency.package_id, dependency.structure)
							)
							.escape_default(),
					);

					let path = dependencies_dir.join(&format!("{alias}.luau").parse().unwrap());

					self.member
						.workspace()
						.vfs()
						.write_file(&path, contents.as_bytes())
						.await
						.map_err(|e| Error::vfs(e, path))?;
				}
			}
		}

		Ok(())
	}

	pub(crate) async fn link_subproject(
		&self,
		dependencies: impl Iterator<Item = (&Alias, &PackageId, &PackageStructure)>,
	) -> Result<()> {
		self.write_config_luau(
			format_args!("member `{}`", self.member.importer()),
			self.member.deser_manifest().await?.config.clone(),
			self.member.importer(),
			dependencies.map(|(alias, package_id, structure)| {
				(alias, self.require_path_for(package_id, structure))
			}),
		)
		.await?;

		Ok(())
	}

	pub(crate) async fn prune_unused_packages(
		&self,
		used_packages: impl Iterator<Item = &PackageId>,
	) -> Result<()> {
		let used_packages: HashSet<_> = used_packages.map(Self::package_dir_name).collect();

		let packages_dir = self.packages_dir();

		let mut read_dir = self.member.workspace().vfs().read_dir(packages_dir.clone());

		while let Some(entry) = read_dir.next().await {
			let entry = match entry {
				Ok(e) => e,
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
				Err(e) => return Err(Error::vfs(e, packages_dir.clone())),
			};

			if !used_packages.contains(&entry.name) {
				self.member
					.workspace()
					.vfs()
					.remove_dir_all(&packages_dir.join(&entry.name))
					.await
					.map_err(|e| Error::vfs(e, entry.name))?;
			}
		}

		Ok(())
	}
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PackageLinkContext<'a> {
	pub package_id: &'a PackageId,
	pub structure: &'a PackageStructure,
	pub context: &'a str,
}

/// A package's layout
#[derive(Debug, Clone)]
pub enum PackageStructure {
	/// Linker files in the parent of the directory containing the package's contents
	Wally(WallyLocalName),
	/// Luau aliases in the directory containing the package's contents
	Pesde(ManifestExports, LuauConfig),
	/// `*_packages` directories inside the package's content directory
	LegacyPesde(ManifestExports, crate::legacy::TargetKind),
}

/// A kind of [PackageStructure].
#[derive(Debug, Clone, Copy)]
pub enum StructureKind {
	/// [PackageStructure::Wally]
	Wally,
	/// [PackageStructure::Pesde]
	Pesde,
	/// [PackageStructure::LegacyPesde]
	LegacyPesde,
}

impl PackageStructure {
	#[must_use]
	pub fn kind(&self) -> StructureKind {
		match self {
			Self::Wally(..) => StructureKind::Wally,
			Self::Pesde(..) => StructureKind::Pesde,
			Self::LegacyPesde(..) => StructureKind::LegacyPesde,
		}
	}
}

impl StructureKind {
	#[must_use]
	pub const fn ignored_directories(self) -> &'static [&'static str] {
		match self {
			Self::Wally => &[],
			Self::Pesde => &["pesde"],
			Self::LegacyPesde => {
				const {
					&[
						TargetKind::Roblox.packages_dir(),
						TargetKind::RobloxServer.packages_dir(),
						TargetKind::Lune.packages_dir(),
						TargetKind::Luau.packages_dir(),
					]
				}
			}
		}
	}

	#[must_use]
	pub const fn ignored_files(self) -> &'static [&'static str] {
		match self {
			Self::Wally => &[],
			Self::Pesde => &[".config.luau"],
			Self::LegacyPesde => &[],
		}
	}
}

#[derive(Debug, Error)]
#[error("failed to serialize Luau config: {0}")]
struct LuauSerializeError(Box<str>);

impl<S: Into<Box<str>>> From<S> for LuauSerializeError {
	fn from(s: S) -> Self {
		Self(s.into())
	}
}

impl serde::ser::Error for LuauSerializeError {
	fn custom<T: std::fmt::Display>(msg: T) -> Self {
		Self(msg.to_string().into())
	}
}

struct LuauSerializer {
	buf: Vec<u8>,
	level: usize,
}

impl<'a> serde::Serializer for &'a mut LuauSerializer {
	type Ok = ();
	type Error = LuauSerializeError;

	type SerializeSeq = LuauCollectionSerializer<'a>;
	type SerializeTuple = serde::ser::Impossible<Self::Ok, Self::Error>;
	type SerializeTupleStruct = serde::ser::Impossible<Self::Ok, Self::Error>;
	type SerializeTupleVariant = serde::ser::Impossible<Self::Ok, Self::Error>;
	type SerializeMap = LuauCollectionSerializer<'a>;
	type SerializeStruct = LuauCollectionSerializer<'a>;
	type SerializeStructVariant = serde::ser::Impossible<Self::Ok, Self::Error>;

	fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
		self.buf
			.extend_from_slice(if v { b"true" } else { b"false" });
		Ok(())
	}

	fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
		self.serialize_f64(v as f64)
	}

	fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
		self.serialize_f64(v as f64)
	}

	fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
		self.serialize_f64(v as f64)
	}

	fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
		const MIN: i64 = -(1 << f64::MANTISSA_DIGITS);
		const MAX: i64 = 1 << f64::MANTISSA_DIGITS;

		if !(MIN..=MAX).contains(&v) {
			return Err("i64 value is too large to serialize as a number".into());
		}
		#[expect(clippy::cast_precision_loss)]
		self.serialize_f64(v as f64)
	}

	fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
		self.serialize_f64(v as f64)
	}

	fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
		self.serialize_f64(v as f64)
	}

	fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
		self.serialize_f64(v as f64)
	}

	fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
		const MAX: u64 = 1 << f64::MANTISSA_DIGITS;
		if v > MAX {
			return Err("u64 value is too large to serialize as a number".into());
		}
		#[expect(clippy::cast_precision_loss)]
		self.serialize_f64(v as f64)
	}

	fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
		self.serialize_f64(v as f64)
	}

	fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
		self.buf.extend_from_slice(v.to_string().as_bytes());
		Ok(())
	}

	fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
		let mut buf = [0; 4];
		let s = v.encode_utf8(&mut buf);
		self.serialize_str(s)
	}

	fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
		self.buf.push(b'"');
		let mut buf = [0; 4];
		for c in v.escape_default() {
			let s = c.encode_utf8(&mut buf);
			self.buf.extend_from_slice(s.as_bytes());
		}
		self.buf.push(b'"');
		Ok(())
	}

	fn serialize_bytes(self, _v: &[u8]) -> Result<Self::Ok, Self::Error> {
		Err("serializing bytes is not supported".into())
	}

	fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
		self.buf.extend_from_slice(b"nil");
		Ok(())
	}

	fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
	where
		T: ?Sized + serde::Serialize,
	{
		value.serialize(self)
	}

	fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
		Err("serializing unit is not supported".into())
	}

	fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
		Err("serializing unit struct is not supported".into())
	}

	fn serialize_unit_variant(
		self,
		_name: &'static str,
		_variant_index: u32,
		_variant: &'static str,
	) -> Result<Self::Ok, Self::Error> {
		Err("serializing unit variant is not supported".into())
	}

	fn serialize_newtype_struct<T>(
		self,
		_name: &'static str,
		value: &T,
	) -> Result<Self::Ok, Self::Error>
	where
		T: ?Sized + serde::Serialize,
	{
		value.serialize(self)
	}

	fn serialize_newtype_variant<T>(
		self,
		_name: &'static str,
		_variant_index: u32,
		_variant: &'static str,
		value: &T,
	) -> Result<Self::Ok, Self::Error>
	where
		T: ?Sized + serde::Serialize,
	{
		value.serialize(self)
	}

	fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
		self.level += 1;
		self.buf.push(b'{');
		Ok(LuauCollectionSerializer {
			serializer: self,
			empty: true,
		})
	}

	fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
		Err("serializing tuple is not supported".into())
	}

	fn serialize_tuple_struct(
		self,
		_name: &'static str,
		_len: usize,
	) -> Result<Self::SerializeTupleStruct, Self::Error> {
		Err("serializing tuple struct is not supported".into())
	}

	fn serialize_tuple_variant(
		self,
		_name: &'static str,
		_variant_index: u32,
		_variant: &'static str,
		_len: usize,
	) -> Result<Self::SerializeTupleVariant, Self::Error> {
		Err("serializing tuple variant is not supported".into())
	}

	fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
		self.level += 1;
		self.buf.push(b'{');
		Ok(LuauCollectionSerializer {
			serializer: self,
			empty: true,
		})
	}

	fn serialize_struct(
		self,
		_name: &'static str,
		_len: usize,
	) -> Result<Self::SerializeStruct, Self::Error> {
		self.level += 1;
		self.buf.push(b'{');
		Ok(LuauCollectionSerializer {
			serializer: self,
			empty: true,
		})
	}

	fn serialize_struct_variant(
		self,
		_name: &'static str,
		_variant_index: u32,
		_variant: &'static str,
		_len: usize,
	) -> Result<Self::SerializeStructVariant, Self::Error> {
		Err("serializing struct variant is not supported".into())
	}
}

struct LuauCollectionSerializer<'a> {
	serializer: &'a mut LuauSerializer,
	empty: bool,
}

impl serde::ser::SerializeSeq for LuauCollectionSerializer<'_> {
	type Ok = ();
	type Error = LuauSerializeError;

	fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
	where
		T: ?Sized + serde::Serialize,
	{
		if self.empty {
			self.serializer.buf.push(b'\n');
		}
		self.empty = false;
		self.serializer
			.buf
			.extend(std::iter::repeat_n(b'\t', self.serializer.level));
		value.serialize(&mut *self.serializer)?;
		self.serializer.buf.extend_from_slice(b",\n");
		Ok(())
	}

	fn end(self) -> Result<Self::Ok, Self::Error> {
		self.serializer.level -= 1;
		if !self.empty {
			self.serializer
				.buf
				.extend(std::iter::repeat_n(b'\t', self.serializer.level));
		}
		self.serializer.buf.push(b'}');
		Ok(())
	}
}

impl serde::ser::SerializeMap for LuauCollectionSerializer<'_> {
	type Ok = ();
	type Error = LuauSerializeError;

	fn serialize_key<T>(&mut self, key: &T) -> Result<(), Self::Error>
	where
		T: ?Sized + serde::Serialize,
	{
		if self.empty {
			self.serializer.buf.push(b'\n');
		}
		self.empty = false;
		self.serializer
			.buf
			.extend(std::iter::repeat_n(b'\t', self.serializer.level).chain(std::iter::once(b'[')));
		key.serialize(&mut *self.serializer)?;
		self.serializer.buf.extend_from_slice(b"] = ");
		Ok(())
	}

	fn serialize_value<T>(&mut self, value: &T) -> Result<(), Self::Error>
	where
		T: ?Sized + serde::Serialize,
	{
		value.serialize(&mut *self.serializer)?;
		self.serializer.buf.extend_from_slice(b",\n");
		Ok(())
	}

	fn end(self) -> Result<Self::Ok, Self::Error> {
		self.serializer.level -= 1;
		if !self.empty {
			self.serializer
				.buf
				.extend(std::iter::repeat_n(b'\t', self.serializer.level));
		}
		self.serializer.buf.push(b'}');
		Ok(())
	}
}

impl serde::ser::SerializeStruct for LuauCollectionSerializer<'_> {
	type Ok = ();
	type Error = LuauSerializeError;

	fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
	where
		T: ?Sized + serde::Serialize,
	{
		serde::ser::SerializeMap::serialize_entry(self, key, value)
	}

	fn end(self) -> Result<Self::Ok, Self::Error> {
		serde::ser::SerializeMap::end(self)
	}
}
