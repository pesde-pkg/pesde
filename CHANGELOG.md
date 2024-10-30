# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.0-rc.7] - 2024-10-30
### Added
- New website by @lukadev-0

### Fixed
- Use updated aliases when reusing lockfile dependencies by @daimond113
- Listen for device flow completion without requiring pressing enter by @daimond113
- Sync scripts repo in background by @daimond113
- Don't make CAS files read-only on Windows (file removal is disallowed if the file is read-only) by @daimond113 
- Validate package names are lowercase by @daimond113

### Performance
- Clone dependency repos shallowly by @daimond113

### Changed
- Optimize boolean expression in `publish` command by @daimond113

## [0.5.0-rc.6] - 2024-10-14
### Added
- Support full version requirements in workspace version field by @daimond113 
- Improved authentication system for registry changes by @daimond113 

### Fixed
- Correct `pesde.toml` inclusion message in `publish` command by @daimond113
- Allow writes to files when `link` is false in PackageFS::write_to by @daimond113
- Handle missing revisions in AnyPackageIdentifier::from_str by @daimond113
- Make GitHub OAuth client ID config optional by @daimond113

## [0.5.0-rc.5] - 2024-10-12
### Added
- Inform user about not finding any bin package when using its bin invocation by @daimond113

### Fixed
- Fix `self-upgrade` overwriting its own binary by @daimond113
- Allow use of Luau packages in `execute` command by @daimond113
- Remove duplicated manifest file name in `publish` command by @daimond113

## [0.5.0-rc.4] - 2024-10-12
### Added
- Add `yes` argument to skip all prompts in publish command by @daimond113
- Publish all workspace members when publishing a workspace by @daimond113

### Fixed
- Add feature gates to `wally-compat` specific code in init command by @daimond113

## [0.5.0-rc.3] - 2024-10-06
### Fixed
- Use workspace specifiers' `target` field when resolving by @daimond113

## [0.5.0-rc.2] - 2024-10-06
### Added
- Add support for multiple targets under the same package name in workspace members by @daimond113
### Fixed
- Fix versions with dots not being handled correctly by @daimond113

## [0.5.0-rc.1] - 2024-10-06
### Changed
- Rewrite the entire project in a more maintainable way by @daimond113
- Support workspaces by @daimond113
- Improve CLI by @daimond113
- Support multiple targets for a single package by @daimond113
- Make registry much easier to self-host by @daimond113
- Start maintaining a changelog by @daimond113

[0.5.0-rc.7]: https://github.com/daimond113/pesde/compare/v0.5.0-rc.6..v0.5.0-rc.7
[0.5.0-rc.6]: https://github.com/daimond113/pesde/compare/v0.5.0-rc.5..v0.5.0-rc.6
[0.5.0-rc.5]: https://github.com/daimond113/pesde/compare/v0.5.0-rc.4..v0.5.0-rc.5
[0.5.0-rc.4]: https://github.com/daimond113/pesde/compare/v0.5.0-rc.3..v0.5.0-rc.4
[0.5.0-rc.3]: https://github.com/daimond113/pesde/compare/v0.5.0-rc.2..v0.5.0-rc.3
[0.5.0-rc.2]: https://github.com/daimond113/pesde/compare/v0.5.0-rc.1..v0.5.0-rc.2
[0.5.0-rc.1]: https://github.com/daimond113/pesde/compare/v0.4.7..v0.5.0-rc.1
