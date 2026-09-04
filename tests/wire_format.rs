// SPDX-License-Identifier: MIT OR Apache-2.0
//! Golden wire-format tests: the exact byte encoding must stay compatible
//! with `suture-protocol`'s delta implementation.
//!
//! The expected byte strings here are hand-derived from the documented
//! format (see crate docs) and were cross-checked against deltas produced
//! by the origin implementation.

use delta_kit::{apply_delta, compute_delta};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn golden_prefix_suffix_delta() -> TestResult {
    // base "Hello, World!" -> target "Hello, Rust!"
    // prefix = "Hello, " (7), suffix = "!" (1), target_len = 12,
    // changed = "Rust".
    let base = b"Hello, World!";
    let target = b"Hello, Rust!";
    let (_c, delta) = compute_delta(base, target);

    let mut expected = Vec::new();
    expected.push(0x01u8);
    expected.extend_from_slice(&7u64.to_le_bytes());
    expected.extend_from_slice(&1u64.to_le_bytes());
    expected.extend_from_slice(&12u64.to_le_bytes());
    expected.extend_from_slice(b"Rust");

    assert_eq!(delta, expected, "0x01 encoding must match the wire spec");
    assert_eq!(apply_delta(base, &delta)?, target.to_vec());
    Ok(())
}

#[test]
fn golden_full_delta() -> TestResult {
    // base = [0,0,0,0] (binary), target = [1,2,3]: no prefix/suffix gain
    // (changed == target) -> full-content encoding.
    let base = [0u8, 0, 0, 0];
    let target = [1u8, 2, 3];
    let (_c, delta) = compute_delta(&base, &target);

    assert_eq!(
        delta,
        vec![0x00, 1, 2, 3],
        "0x00 encoding must match the wire spec"
    );
    assert_eq!(apply_delta(&base, &delta)?, target.to_vec());
    Ok(())
}

#[test]
fn golden_instruction_stream_layout() -> TestResult {
    // Hand-assemble a 0x02 delta per the documented layout and confirm
    // the decoder accepts exactly this byte arrangement.
    // base = "ABCDEFGH", target = base[0..4] ++ "XY" ++ base[4..8] —
    // wait, concretely: Copy(0,4)="ABCD", Insert("XY"), Copy(4,4)="EFGH"
    // reconstructs "ABCDXYEFGH" (10 bytes).
    let base = b"ABCDEFGH";
    let mut d = Vec::new();
    d.push(0x02);
    d.extend_from_slice(&10u64.to_le_bytes()); // target_len
    d.extend_from_slice(&3u32.to_le_bytes()); // instruction count
    d.push(0x01); // Copy
    d.extend_from_slice(&0u64.to_le_bytes());
    d.extend_from_slice(&4u32.to_le_bytes()); // "ABCD"
    d.push(0x02); // Insert
    d.extend_from_slice(&2u32.to_le_bytes());
    d.extend_from_slice(b"XY");
    d.push(0x01); // Copy
    d.extend_from_slice(&4u64.to_le_bytes());
    d.extend_from_slice(&4u32.to_le_bytes()); // "EFGH"

    let out = apply_delta(base, &d)?;
    assert_eq!(out, b"ABCDXYEFGH".to_vec());
    Ok(())
}

#[test]
fn golden_instruction_header_is_thirteen_bytes() -> TestResult {
    // Copy records must be exactly 1 + 8 + 4 = 13 bytes and Insert records
    // exactly 1 + 4 + n: verify by concatenating one of each and decoding.
    let base = b"0123456789";
    let mut d = Vec::new();
    d.push(0x02);
    d.extend_from_slice(&9u64.to_le_bytes()); // target_len: "01" + "Q" + "456789"
    d.extend_from_slice(&3u32.to_le_bytes());
    d.push(0x01);
    d.extend_from_slice(&0u64.to_le_bytes());
    d.extend_from_slice(&2u32.to_le_bytes()); // "01"
    d.push(0x02);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.push(b'Q'); // "Q"
    d.push(0x01);
    d.extend_from_slice(&4u64.to_le_bytes());
    d.extend_from_slice(&6u32.to_le_bytes()); // "456789"

    let out = apply_delta(base, &d)?;
    assert_eq!(out, b"01Q456789".to_vec());
    Ok(())
}

#[test]
fn base_copy_returned_by_compute() {
    // Compatibility: compute_delta returns (copy of base, delta).
    let base = b"keep me";
    let target = b"keep me too";
    let (base_copy, _delta) = compute_delta(base, target);
    assert_eq!(base_copy, base.to_vec());
}
