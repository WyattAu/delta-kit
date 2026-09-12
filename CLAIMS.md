# delta-kit — claims inventory

Every verifiable numeric / behavioral performance claim in README.md and the
crate docs, mapped to its proof artifact. Generated as part of the
perf-claims proof-back pass (0.2.1).

Status legend:

- **backed** — an existing bench/test asserts the claim; linked below.
- **proven** — unproven at survey time; a bench/test was added by this pass.
- **code-backed** — enforced structurally in the code (checked by build, not
  by a runtime assertion).

## Correctness claims that bound the codec's behavior

| Claim | Proof artifact | Status |
|---|---|---|
| Roundtrip: `apply_delta` reproduces the target for any produced delta (REQ-DK-001) | `tests/proptest.rs::roundtrip`, `::rolling_path_roundtrip`, `::binary_path_roundtrip` + unit tests | backed |
| Binary safety: zero bytes in inputs never break the roundtrip | `tests/proptest.rs::binary_safety` | backed |
| Universal size bound: a delta is never meaningfully larger than the target | `tests/proptest.rs::delta_size_bounded` | backed |
| Similar inputs yield a small delta (the codec's reason to exist) | `tests/proptest.rs::similar_inputs_small_delta` | backed |
| Delta-of-delta composes (applying to an already-patched base works) | `tests/proptest.rs::delta_of_delta_composes` | backed |
| Wire format is byte-stable (golden bytes, origin `suture-protocol` compat) | `tests/wire_format.rs` (5 golden tests) | backed |
| Strategy selection picks the smallest applicable encoding (REQ-DK-002) | unit tests in `src/delta.rs` (strategy selection) | backed |
| `compute_delta` is total; its output always applies | doc contract + all roundtrip properties above | backed |
| Malformed deltas return typed errors, never panic (REQ-DK-100) | unit tests (hardened-decode errors in `src/delta.rs`) + `tests/wire_format.rs` | backed |
| Without the `zstd` feature, `0x03` deltas are never produced and fail decode | unit tests (`ZstdDisabled`) | backed |
| `no_std` + `alloc` core build (`--no-default-features`, REQ-DK-003) | `cargo check --no-default-features` in CI; `std` gate in `src/lib.rs` | code-backed |

## Resource-bound claims

| Claim | Proof artifact | Status |
|---|---|---|
| Apply never over-allocates beyond the declared `target_len` (REQ-DK-100): O(1) buffer allocations independent of the instruction count (10k- and 40k-instruction deltas stay in the same ≤8-allocation budget) | `tests/alloc_bounds.rs` (counting global allocator) | **proven** |
| Decompression output is bounded on the `0x03` path (REQ-DK-101) | zstd limit in `src/delta.rs` + unit tests | backed |

## Hot-path instruction counts (new in 0.2.1)

The deterministic regression gate is `benches/iai_delta.rs`
([iai-callgrind](https://github.com/iai-callgrind/iai-callgrind); CI-only,
requires valgrind). Baseline on a 128 KiB similar pair (4 KiB scramble in
the middle, rolling-hash strategy):

| Path | Instructions (baseline) |
|---|---|
| `encode_rolling` (block index + rolling-window scan) | ~4.66 M |
| `apply_rolling` (Copy/Insert reconstruction) | ~129 K |

## Wall-clock trend (criterion, indicative)

Development machine, x86-64, 64 KiB inputs (`benches/delta_bench.rs`):

| Bench | Result |
|---|---|
| `delta_text_edit_64k` (1 KiB changed mid-file) | ~390 µs |
| `delta_binary_edit_64k` (1 KiB changed, binary-ish) | ~114 µs |
| `delta_append_64k` (2 KiB appended) | ~113 µs |

## Summary

- Backed by existing artifacts: 12 (plus 2 code-backed)
- Proven by artifacts added in this pass: 1 (apply allocation bound)
- Reworded or deleted: 0

New proof artifacts added in 0.2.1:

- `benches/iai_delta.rs` — iai-callgrind instruction-count gate for the
  rolling-hash encode loop and the Copy/Insert apply loop.
- `tests/alloc_bounds.rs` — counting-allocator proof that `apply_delta` /
  `apply_delta_lenient` allocate O(1) buffers bounded by the declared
  `target_len`, independent of the instruction count.
