# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

## [0.2.0] - 2026-09-07

### Added
- `no_std` + `alloc` support: rolling-hash delta and apply paths build
  core-only with `--no-default-features` (thumbv7em-none-eabihf passes
  `cargo check`). New `std` feature (no-op); `zstd` implies `std` since
  zstd compression is std-bound.
- `apply_delta_lenient`: infallible, behavior-compatible port of the
  origin `suture-protocol` decoder — silent repair of malformed deltas
  (echo on unknown opcode/truncated header/zstd failure, skip
  out-of-range records, empty vector on checksum mismatch) — for
  consumers preserving the legacy `apply_delta -> Vec<u8>` contract.
- `compute_binary_delta` is now public: compute a `0x03` XOR+Zstd delta
  directly, bypassing the binary-sniffing gate (requires `zstd`).

### Changed
- `blake3`/`thiserror` build without std default features; `HashMap` comes
  from `hashbrown` (alloc has no HashMap).

## [0.1.0] - 2026-09-05

### Added
- Initial public release.
