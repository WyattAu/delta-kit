# Threat Model — delta-kit

Reference: STRIDE. Scope: the crate's public API surface (`compute_delta`,
`apply_delta`, `apply_delta_lenient`, `compute_binary_delta`) as used by a
downstream sync protocol (e.g. `suture-protocol`). Trust boundaries:
(1) untrusted delta bytes entering `apply_delta`/`apply_delta_lenient`,
(2) base/target byte strings from peers, (3) the dependency tree (blake3,
zstd).

## Assets

| ID | Asset | Example |
|----|-------|---------|
| A1 | Availability of the decoding process | Malformed delta aborts the sync worker (allocation blowup or panic) |
| A2 | Integrity of reconstructed targets | Corrupted delta silently yields wrong bytes |

## STRIDE Analysis

| # | Threat | Category | Surface | Mitigation | Verifying test |
|---|--------|----------|---------|------------|----------------|
| T1 | Panic/OOB on malformed delta streams | DoS | `apply_delta` | Strict hardened decoding (the origin's silent-repair behavior was removed): truncated headers, unknown opcodes/instructions, out-of-range copies, and inserts past end all return `DeltaError` — never panic (`#![forbid(unsafe_code)]`) | `test_apply_truncated_headers_are_errors`, `test_apply_unknown_opcode_is_error`, `test_apply_unknown_instruction_is_error`, `test_apply_copy_out_of_range_is_error`, `test_apply_insert_past_end_is_error`, `test_apply_empty_delta_is_error`; proptests `binary_safety`, `roundtrip` (`tests/proptest.rs`) |
| T2 | Corrupted binary delta accepted (XOR path) | Tampering | `apply_delta` opcode `0x03` | BLAKE3 16-byte digests of base and target embedded in the frame; mismatch fails with an error instead of returning wrong bytes | `test_apply_binary_checksum_mismatch_is_error`, `test_binary_delta_tamper_is_error` |
| T3 | Declared-length lie causes huge allocation | DoS | `apply_delta` opcodes `0x01`/`0x02` | **Not fully mitigated** — `Vec::with_capacity(declared_target_len)` trusts the header before instruction validation, so a tiny delta declaring `u64::MAX` target length can trigger an oversized allocation → abort. The final length check happens *after* reconstruction. Documented residual risk | `test_apply_target_length_mismatch_is_error` (correctness check); no allocation cap test exists |
| T4 | Tampered delta on the *non-binary* paths (opcodes 0x00–0x02) | Tampering | `apply_delta` | **Not mitigated** — plain opcodes carry no checksum at all; the declared-target-length check catches only *structural* corruption. Documented: integrity for these paths is the transport's job | Code review |
| T5 | Checksum treated as authentication | Spoofing | opcode `0x03` | Explicitly *not* claimed: BLAKE3 here is unkeyed — a tamper-detection checksum, not a MAC. A malicious peer can recompute valid digests for modified content. Documented: peer authentication belongs to the transport | Crate docs ("byte-compatible with suture-protocol"); no MAC exists by design |
| T6 | `apply_delta_lenient` silently swallows corruption | Tampering | `apply_delta_lenient` | Kept for byte-compat with the origin's observable behavior and explicitly documented as the unsafe-for-untrusted-input variant; checksum mismatch returns empty, truncated headers echo the delta | `test_lenient_checksum_mismatch_returns_empty`, `test_lenient_skips_out_of_range_copy`, `test_lenient_truncated_prefix_suffix_echoes_delta` |

## Repudiation

Not applicable — stateless codec, no logs, no identity.

## Out of Scope

- Compression-bomb depth for the embedded zstd frame: `zstd` crate limits
  apply; no delta-specific output-size cap exists (see R1).
- Transport authenticity and causal ordering of deltas in a sync session
  (protocol layer, e.g. `suture-protocol`).
- `compute_delta` DoS: computing is caller-driven over caller-provided
  buffers; no adversarial amplification exists beyond O(target + base).

## Residual Risks

- **R1 (Medium, accepted):** Attacker-declared target length drives
  `with_capacity` before validation (T3). Recommend capping declared length
  against a function of `delta.len()` and `base.len()` before allocating —
  tracked as the top open hardening item.
- **R2 (Low, accepted):** 16-byte truncated BLAKE3 gives 128-bit collision
  resistance for tamper detection — comfortably sufficient, but reduced from
  the full 256-bit digest; fine for accidental corruption, irrelevant for
  adversarial cases (which R3/T5 already delegate to the transport).
- **R3 (Low, accepted):** Lenient mode is a loaded foot-gun: callers
  accepting deltas from untrusted peers must use `apply_delta`, never
  `apply_delta_lenient`. Naming and docs carry the warning; no type-level
  separation.
- **R4 (Low, accepted):** Dependency risk in `blake3`/`zstd`; no in-repo
  `cargo audit` gate.
