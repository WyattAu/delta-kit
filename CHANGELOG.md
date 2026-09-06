# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

### Added
- `no_std` + `alloc` support: rolling-hash delta and apply paths build
  core-only with `--no-default-features` (thumbv7em-none-eabihf passes
  `cargo check`). New `std` feature (no-op); `zstd` implies `std` since
  zstd compression is std-bound.

### Changed
- `blake3`/`thiserror` build without std default features; `HashMap` comes
  from `hashbrown` (alloc has no HashMap).

## [0.1.0] - 2026-09-05

### Added
- Initial public release.
