# PATTERN_ADOPTION

Relationship declaration to the WyattAu pattern canon
(pattern-standards): format per references/adoption-format.md in that
canon, gated by its adoption-check.py script.

Scope: a pure binary delta codec. Patterns outside this domain
(distributed state, concurrency control, service reliability tiers) are
omitted per rule 6 of the format.

| Pattern | Status | Evidence |
|---|---|---|
| T1 | adopted | src/delta.rs:456 (untrusted wire bytes parse into typed instructions once; malformed input yields DeltaError instead of the origin's silent repair; parse helpers at src/delta.rs:408) |
| T13 | adopted | src/lib.rs:2 (whole crate is a pure core: total functions, zero I/O, no mock seams needed in tests) |
| T28 | partial | src/delta.rs:208 (0x03 format embeds blake3(base) and blake3(target) prefixes, verified at apply by src/delta.rs:110; operand hashing only, no Merkle DAG) |
| P64 | adopted | tests/proptest.rs:28 (proptest block at 256 cases: roundtrip, binary safety, size bounds, delta-of-delta composition) |
| P65 | adopted | tests/wire_format.rs:3 (golden bytes cross-checked against the origin suture-protocol implementation) |
| P96 | partial | src/delta.rs:589 (declared target_len checked against the reconstruction at every apply stage; checksum gate at src/delta.rs:110; decoder scope only) |

No refused rows: no pattern was consciously rejected with a recorded
decision. No planned rows: no dated tracking reference exists yet.
