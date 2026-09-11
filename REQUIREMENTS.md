# Requirements — delta-kit

Numbered, testable requirements. Every requirement maps to at least one named
test or doc-comment contract; security-relevant items cite THREAT-MODEL.md rows.

Scope: Binary deltas — rolling-hash chunking with optional XOR+Zstd binary path

## Functional

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-DK-001 | `apply_delta(base, delta)` reproduces `target` exactly for any produced delta (roundtrip property) | MUST |
| REQ-DK-002 | Encoding selection (prefix/suffix vs binary XOR+Zstd) picks the smallest applicable encoding | MUST |
| REQ-DK-003 | Zstd path is feature-gated; default builds remain `no_std`-capable | MUST |

## Security

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-DK-100 | Delta application validates opcodes/lengths; malformed deltas return errors, never panic or over-allocate beyond declared limits | MUST |
| REQ-DK-101 | Decompression output is bounded (Zstd limit applied) | MUST |

## Observability & API hygiene

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-DK-900 | All fallible public APIs return typed errors; production `unwrap`/`expect` is denied or explicitly justified with an invariant comment | MUST |
| REQ-DK-901 | Public items carry doc comments with runnable examples where practical | SHOULD |

Reviewed: 2026-09-11
