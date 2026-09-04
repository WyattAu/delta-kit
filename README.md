# delta-kit

A binary-safe delta codec for Rust: compute a compact delta between two
byte strings and apply it to reconstruct the target. Byte-compatible wire
format with the `suture-protocol` crate it was extracted from.

Extracted from the [Suture](https://github.com/WyattAu/suture) codebase,
with decoding hardened from silent fallbacks to proper errors.

## Strategies

`compute_delta(base, target)` picks the smallest applicable encoding:

| Opcode | Strategy | When |
|--------|----------|------|
| `0x03` | XOR + Zstd | Either input looks binary (zero byte in first 8 KiB) and the XOR stream compresses smaller (`zstd` feature) |
| `0x02` | Rabin rolling hash | Both inputs ≥ 4 KiB (fixed 4 KiB blocks indexed by Rabin hash, base 257 over Mersenne 2^61−1, verified by a strong 64-bit hash; target scanned with a rolling window emitting Copy/Insert opcodes) |
| `0x01` | Prefix/suffix trim | Small inputs; common prefix + suffix stored implicitly, changed middle kept |
| `0x00` | Full content | Fallback |

`apply_delta(base, delta)` reconstructs the target.

## Usage

```rust
let base = b"Hello, World!";
let target = b"Hello, Rust!";

let (_base_copy, delta) = delta_kit::compute_delta(base, target);
assert_eq!(
    delta_kit::apply_delta(base, &delta).expect("compute_delta output always applies"),
    target.to_vec()
);
```

`compute_delta` is total (cannot fail) and always produces a delta that
`apply_delta` accepts. The returned first element is a copy of `base`,
kept for drop-in compatibility with `suture-protocol::compute_delta`.

## Wire format

All integers little-endian; first byte selects the encoding:

```text
0x00 full content : [1..] target
0x01 prefix/suffix: [1..9) u64 prefix_len, [9..17) u64 suffix_len,
                    [17..25) u64 target_len, [25..] changed middle bytes
0x02 instructions : [1..9) u64 target_len, [9..13) u32 count,
                    records: 0x01 + u64 base_offset + u32 len  (Copy)
                             0x02 + u32 len + bytes            (Insert)
0x03 binary XOR   : [1..9) u64 target_len, [9..25) blake3(base)[..16],
                    [25..41) blake3(target)[..16], [41..] zstd(xor)
```

## Hardened decoding (vs. origin)

The origin `suture-protocol` silently repaired malformed deltas:
truncated headers decoded as identity, out-of-range copies were skipped,
checksum failures returned an empty vector. This crate returns
`DeltaError` for all of those (`Truncated`, `InvalidOpcode`,
`InvalidInstruction`, `CopyOutOfRange`, `InsertOutOfRange`,
`TargetLengthMismatch`, `ChecksumMismatch`). Well-formed deltas decode
identically, and compatibility is verified byte-for-byte against the
origin implementation across all four encodings.

## Cargo features

| Feature | Default | Effect |
|---------|---------|--------|
| `zstd`  | yes     | Enables the `0x03` binary XOR+Zstd strategy. Without it, `0x03` deltas fail to decode (`ZstdDisabled`) and are never produced. |

## Testing

- 32 tests: unit (strategy coverage, hardened-decode errors), golden
  wire-format bytes, and property-based tests (arbitrary-bytes roundtrip,
  binary safety with forced zeros, universal size bound, similar-input
  bound, delta-of-delta composition).
- `cargo check --no-default-features` verified.
- `cargo clippy -D warnings` and `cargo fmt --check` clean.

## License

MIT OR Apache-2.0
