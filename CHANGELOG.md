# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Changed
- Allow publishing other packages even if an error occurred by @daimond113

## [0.6.0-rc.7] - 2025-02-14
### Changed
- Make aliases case-insensitive by @daimond113
- Print "update available" message to stderr by @daimond113
- Improve output of the `outdated` command by @daimond113

## [0.6.0-rc.6] - 2025-02-10
### Fixed
- Fix double path long prefix issues on Windows by @daimond113

## [0.6.0-rc.5] - 2025-02-10
### Fixed
- Correct script linker require paths on Windows by @daimond113
- Improve patches in incremental installs by @daimond113
- Patches now include newly created files by @daimond113

### Changed
- Patches are now applied before type extraction to allow patches to modify types by @daimond113

## [0.6.0-rc.4] - 2025-02-08
### Fixed
- Refresh sources before reading package data to ensure the index is even cloned (remote changes to lockfile) by @daimond113

## [0.6.0-rc.3] - 2025-02-08
### Fixed
- Fix `self-upgrade` using the wrong path when doing a fresh download by @daimond113
- Fix types not being re-exported by @daimond113

## [0.6.0-rc.2] - 2025-02-07
### Fixed
- Colour deprecate output to match yank output by @daimond113
- Fix zbus panic on Linux by @daimond113

## [0.6.0-rc.1] - 2025-02-06
### Added
- Improve installation experience by @lukadev-0
- Support using aliases of own dependencies for overrides by @daimond113
- Support ignoring parse errors in Luau files by @daimond113
- Add path dependencies by @daimond113
- Inherit pesde-managed scripts from workspace root by @daimond113
- Allow using binaries from workspace root in member packages by @daimond113
- Add yanking & deprecating by @daimond113
- Add engines as a form of managing runtimes by @daimond113
- Modify existing installed packages instead of always reinstalling by @daimond113
- Add `cas prune` command to remove unused CAS files & packages by @daimond113
- Add `list` and `remove` commands to manage packages in the manifest by @daimond113

### Fixed
- Install dev packages in prod mode and remove them after use to allow them to be used in scripts by @daimond113
- Fix infinite loop in the resolver in packages depending on themselves by @daimond113
- Do Git operations inside spawn_blocking to avoid performance issues by @daimond113
- Scope CAS package indices to the source by @daimond113
- Do not copy `default.project.json` in workspace dependencies by @daimond113

### Changed
- Change handling of graphs to a flat structure by @daimond113
- Store dependency over downloaded graphs in the lockfile by @daimond113
- Improve linking process by @daimond113
- Use a proper url encoding library to ensure compatibility with all characters by @daimond113
- The `*` specifier now matches all versions, even prereleases by @daimond113
- Switch CLI dependencies to ones used by other dependencies to optimize the binary size by @daimond113
- Reorder the `help` command by @daimond113
- Ignore submodules instead of failing when using Git dependencies with submodules by @daimond113
- Exit with code 1 from invalid directory binary linkers by @daimond113

### Removed
- Remove old includes format compatibility by @daimond113
- Remove data redundancy for workspace package references by @daimond113
- Remove dependency checks from CLI in publish command in favor of registry checks by @daimond113

### Performance
- Use `Arc` for more efficient cloning of multiple structs by @daimond113
- Avoid cloning where possible by @daimond113
- Remove unnecessary mutex in Wally package download by @daimond113
- Lazily format error messages by @daimond113

## [0.5.3] - 2024-12-30
### Added
- Add meta field in index files to preserve compatibility with potential future changes by @daimond113

### Changed
- Remove verbosity from release mode logging by @daimond113

## [0.5.2] - 2024-12-19
### Fixed
- Change dependency types for removed peer dependencies by @daimond113
- Resolve version to correct tag for `pesde_version` field by @daimond113
- Do not error on missing dependencies until full linking by @daimond113

### Changed
- Switch from `log` to `tracing` for logging by @daimond113

## [0.5.1] - 2024-12-15
### Fixed
- Ignore build metadata when comparing CLI versions by @daimond113

## [0.5.0] - 2024-12-14
### Added
- Add support for multiple targets under the same package name in workspace members by @daimond113
- Add `yes` argument to skip all prompts in publish command by @daimond113
- Publish all workspace members when publishing a workspace by @daimond113
- Inform user about not finding any bin package when using its bin invocation by @daimond113
- Support full version requirements in workspace version field by @daimond113
- Improved authentication system for registry changes by @daimond113
- New website by @lukadev-0
- Add `--index` flag to `publish` command to publish to a specific index by @daimond113
- Support fallback Wally registries by @daimond113
- Print that no updates are available in `outdated` command by @daimond113
- Support negated globs in `workspace_members` field by @daimond113
- Make `includes` use glob patterns by @daimond113
- Use symlinks for workspace dependencies to not require reinstalling by @daimond113
- Add `auth token` command to print the auth token for the index by @daimond113
- Support specifying which external registries are allowed on registries by @daimond113
- Add improved CLI styling by @daimond113
- Install pesde dependencies before Wally to support scripts packages by @daimond113
- Support packages exporting scripts by @daimond113
- Support using workspace root as a member by @daimond113
- Allow multiple, user selectable scripts packages to be selected (& custom packages inputted) in `init` command by @daimond113
- Support granular control over which repositories are allowed in various specifier types by @daimond113
- Display included scripts in `publish` command by @daimond113

### Fixed
- Fix versions with dots not being handled correctly by @daimond113
- Use workspace specifiers' `target` field when resolving by @daimond113
- Add feature gates to `wally-compat` specific code in init command by @daimond113
- Remove duplicated manifest file name in `publish` command by @daimond113
- Allow use of Luau packages in `execute` command by @daimond113
- Fix `self-upgrade` overwriting its own binary by @daimond113
- Correct `pesde.toml` inclusion message in `publish` command by @daimond113
- Allow writes to files when `link` is false in PackageFS::write_to by @daimond113
- Handle missing revisions in AnyPackageIdentifier::from_str by @daimond113
- Make GitHub OAuth client ID config optional by @daimond113
- Use updated aliases when reusing lockfile dependencies by @daimond113
- Listen for device flow completion without requiring pressing enter by @daimond113
- Sync scripts repo in background by @daimond113
- Don't make CAS files read-only on Windows (file removal is disallowed if the file is read-only) by @daimond113
- Validate package names are lowercase by @daimond113
- Use a different algorithm for finding a CAS directory to avoid issues with mounted drives by @daimond113
- Remove default.project.json from Git pesde dependencies by @daimond113
- Correctly (de)serialize workspace specifiers by @daimond113
- Fix CAS finder algorithm issues with Windows by @daimond113
- Fix CAS finder algorithm's AlreadyExists error by @daimond113
- Use moved path when setting file to read-only by @daimond113
- Correctly link Wally server packages by @daimond113
- Fix `self-install` doing a cross-device move by @daimond113
- Add back mistakenly removed updates check caching by @daimond113
- Set download error source to inner error to propagate the error by @daimond113
- Correctly copy workspace packages by @daimond113
- Fix peer dependencies being resolved incorrectly by @daimond113
- Set PESDE_ROOT to the correct path in `pesde run` by @daimond113
- Install dependencies of packages in `x` command by @daimond113
- Fix `includes` not supporting root files by @daimond113
- Link dependencies before type extraction to support more use cases by @daimond113
- Strip `.luau` extension from linker modules' require paths to comply with Luau by @daimond113
- Correctly handle graph paths for resolving overriden packages by @daimond113
- Do not require `--` in bin package executables on Unix by @daimond113
- Do not require lib or bin exports if package exports scripts by @daimond113
- Correctly resolve URLs in `publish` command by @daimond113
- Add Roblox types in linker modules even with no config generator script by @daimond113

### Removed
- Remove special scripts repo handling to favour standard packages by @daimond113

### Changed
- Rewrite the entire project in a more maintainable way by @daimond113
- Support workspaces by @daimond113
- Improve CLI by @daimond113
- Support multiple targets for a single package by @daimond113
- Make registry much easier to self-host by @daimond113
- Start maintaining a changelog by @daimond113
- Optimize boolean expression in `publish` command by @daimond113
- Switched to fs-err for better errors with file system operations by @daimond113
- Use body bytes over multipart for publishing packages by @daimond113
- `self-upgrade` now will check for updates by itself by default by @daimond113
- Only store `pesde_version` executables in the version cache by @daimond113
- Remove lower bound limit of 3 characters for pesde package names by @daimond113

### Performance
- Clone dependency repos shallowly by @daimond113
- Switch to async Rust by @daimond113
- Asyncify dependency linking by @daimond113
- Use `exec` in Unix bin linking to reduce the number of processes by @daimond113

[0.6.0-rc.7]: https://github.com/daimond113/pesde/compare/v0.6.0-rc.6%2Bregistry.0.2.0-rc.2..v0.6.0-rc.7%2Bregistry.0.2.0-rc.3
[0.6.0-rc.6]: https://github.com/daimond113/pesde/compare/v0.6.0-rc.5%2Bregistry.0.2.0-rc.2..v0.6.0-rc.6%2Bregistry.0.2.0-rc.2
[0.6.0-rc.5]: https://github.com/daimond113/pesde/compare/v0.6.0-rc.4%2Bregistry.0.2.0-rc.1..v0.6.0-rc.5%2Bregistry.0.2.0-rc.2
[0.6.0-rc.4]: https://github.com/daimond113/pesde/compare/v0.6.0-rc.3%2Bregistry.0.2.0-rc.1..v0.6.0-rc.4%2Bregistry.0.2.0-rc.1
[0.6.0-rc.3]: https://github.com/daimond113/pesde/compare/v0.6.0-rc.2%2Bregistry.0.2.0-rc.1..v0.6.0-rc.3%2Bregistry.0.2.0-rc.1
[0.6.0-rc.2]: https://github.com/daimond113/pesde/compare/v0.6.0-rc.1%2Bregistry.0.2.0-rc.1..v0.6.0-rc.2%2Bregistry.0.2.0-rc.1
[0.6.0-rc.1]: https://github.com/daimond113/pesde/compare/v0.5.3%2Bregistry.0.1.2..v0.6.0-rc.1%2Bregistry.0.2.0-rc.1
[0.5.3]: https://github.com/daimond113/pesde/compare/v0.5.2%2Bregistry.0.1.1..v0.5.3%2Bregistry.0.1.2
[0.5.2]: https://github.com/daimond113/pesde/compare/v0.5.1%2Bregistry.0.1.0..v0.5.2%2Bregistry.0.1.1
[0.5.1]: https://github.com/daimond113/pesde/compare/v0.5.0%2Bregistry.0.1.0..v0.5.1%2Bregistry.0.1.0
[0.5.0]: https://github.com/daimond113/pesde/compare/v0.4.7..v0.5.0%2Bregistry.0.1.0
