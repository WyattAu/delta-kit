# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

## [0.2.1] - 2026-09-12

### Added

- Perf-claims proof-back pass: `CLAIMS.md` maps every README/docs claim to
  its proof artifact (12 backed + 2 code-backed, 1 newly proven, 0 removed).
- `benches/iai_delta.rs`: iai-callgrind instruction-count regression gate
  for the hot loops — `encode_rolling` (block-index build + rolling-window
  scan over a 128 KiB similar pair) and `apply_rolling` (Copy/Insert
  reconstruction). CI-only execution (needs valgrind); compiles everywhere.
- `tests/alloc_bounds.rs`: counting-global-allocator proof that
  `apply_delta` / `apply_delta_lenient` allocate O(1) buffers bounded by
  the declared `target_len`, independent of the instruction count (10k-
  and 40k-instruction deltas share the same ≤8-allocation budget) —
  turning REQ-DK-100's "never over-allocates beyond declared limits"
  clause into a verified invariant on every `cargo test` run.

### Changed

- README: new "Benchmarks" section quoting the measured baseline, the
  instruction gate, and the alloc bound; links `CLAIMS.md`. No API changes.

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
