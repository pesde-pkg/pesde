# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2025-03-02
### Changed
- Print more error info by @daimond113

## [0.2.0] - 2025-02-22
### Added
- Support deprecating and yanking packages by @daimond113
- Add yanking & deprecating to registry by @daimond113
- Log more information about configured auth & storage by @daimond113
- Add individual endpoints for package data over using `Accept` header conditional returns by @daimond113
- Set `Content-Length` header for FS storage backend by @daimond113

### Changed
- Remove native-tls dependency by @daimond113
- Make aliases case-insensitive by @daimond113

### Performance
- Switch to using a `RwLock` over a `Mutex` to store repository data by @daimond113
- Asyncify blocking operations by @daimond113
- Asyncify reading of package data of top search results by @daimond113

## [0.1.2] - 2024-12-30
### Changed
- Update to pesde lib API changes by @daimond113

## [0.1.1] - 2024-12-19
### Changed
- Switch to tracing for logging by @daimond113

## [0.1.0] - 2024-12-14
### Added
- Rewrite registry for pesde v0.5.0 by @daimond113

[0.2.1]: https://github.com/daimond113/pesde/compare/v0.6.0%2Bregistry.0.2.0..v0.6.0%2Bregistry.0.2.1
[0.2.0]: https://github.com/daimond113/pesde/compare/v0.5.3%2Bregistry.0.1.2..v0.6.0%2Bregistry.0.2.0
[0.1.2]: https://github.com/daimond113/pesde/compare/v0.5.2%2Bregistry.0.1.1..v0.5.3%2Bregistry.0.1.2
[0.1.1]: https://github.com/daimond113/pesde/compare/v0.5.1%2Bregistry.0.1.0..v0.5.2%2Bregistry.0.1.1
[0.1.0]: https://github.com/daimond113/pesde/compare/v0.4.7..v0.5.0%2Bregistry.0.1.0
