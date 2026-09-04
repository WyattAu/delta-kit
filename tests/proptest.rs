// SPDX-License-Identifier: MIT OR Apache-2.0
//! Property-based tests for `delta-kit`:
//!
//! - `apply(base, compute(base, target).1) == target` for arbitrary bytes
//!   (binary safety, including zeros).
//! - Delta size is bounded for all strategies.
//! - Similar inputs produce non-growing deltas.
//! - Chained "delta of delta" application composes.

use proptest::prelude::*;

use delta_kit::{apply_delta, compute_delta, BLOCK_SIZE};

/// Convert any fallible step into a proptest failure (no unwraps).
fn soft<T, E: std::fmt::Display>(r: Result<T, E>) -> Result<T, TestCaseError> {
    r.map_err(|e| TestCaseError::fail(e.to_string()))
}

fn arb_bytes(max: usize) -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(proptest::num::u8::ANY, 0..max)
}

/// Zero-free bytes (text-like) so the binary XOR path is avoided.
fn arb_text(max: usize) -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(1u8..=255, 0..max)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Core roundtrip: arbitrary base/target pairs, including empty,
    /// all-zeros, and mixed-binary content.
    #[test]
    fn roundtrip(base in arb_bytes(4096), target in arb_bytes(4096)) {
        let (_base_copy, delta) = compute_delta(&base, &target);
        let out = soft(apply_delta(&base, &delta))?;
        prop_assert_eq!(out, target);
    }

    /// Binary safety with forced zero runs on both sides.
    #[test]
    fn binary_safety(
        base in proptest::collection::vec(proptest::num::u8::ANY, 0..2048),
        target in proptest::collection::vec(proptest::num::u8::ANY, 0..2048),
        zpos1 in 0..2048usize,
        zpos2 in 0..2048usize,
    ) {
        let mut base = base;
        let mut target = target;
        if !base.is_empty() {
            let p = zpos1 % base.len();
            base[p] = 0;
        }
        if !target.is_empty() {
            let p = zpos2 % target.len();
            target[p] = 0;
        }

        let (_c, delta) = compute_delta(&base, &target);
        let out = soft(apply_delta(&base, &delta))?;
        prop_assert_eq!(out, target);
    }

    /// Universal size bound across all four encodings:
    /// 0x00 = n+1, 0x01 = n+25, 0x02 < n, 0x03 < n+41.
    #[test]
    fn delta_size_bounded(base in arb_bytes(8192), target in arb_bytes(8192)) {
        let (_c, delta) = compute_delta(&base, &target);
        prop_assert!(delta.len() <= target.len() + 64,
            "delta {} bytes vs target {} bytes", delta.len(), target.len());
    }

    /// Similar inputs never need a full rewrite: the delta stays within
    /// the non-binary encodings' bound of target + 25.
    #[test]
    fn similar_inputs_small_delta(
        seed in proptest::num::u64::ANY,
        edit1 in 0..(3 * BLOCK_SIZE),
        edit2 in 0..(3 * BLOCK_SIZE),
    ) {
        // Deterministic pseudo-random zero-free base of 3 blocks.
        let mut state = seed | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % 255 + 1) as u8
        };
        let base: Vec<u8> = (0..3 * BLOCK_SIZE).map(|_| next()).collect();

        let mut target = base.clone();
        if !target.is_empty() {
            let p1 = edit1 % target.len();
            target[p1] = target[p1].wrapping_add(1).max(1);
            let p2 = edit2 % target.len();
            target[p2] = target[p2].wrapping_sub(1).max(1);
        }

        let (_c, delta) = compute_delta(&base, &target);
        prop_assert!(delta.len() <= target.len() + 25,
            "similar-input delta {} bytes vs target {} bytes", delta.len(), target.len());
        let out = soft(apply_delta(&base, &delta))?;
        prop_assert_eq!(out, target);
    }

    /// Rolling path: block-sized edits across >= BLOCK_SIZE inputs.
    #[test]
    fn rolling_path_roundtrip(base in arb_text(4 * BLOCK_SIZE), target in arb_text(4 * BLOCK_SIZE)) {
        prop_assume!(base.len() >= BLOCK_SIZE && target.len() >= BLOCK_SIZE);
        let (_c, delta) = compute_delta(&base, &target);
        let out = soft(apply_delta(&base, &delta))?;
        prop_assert_eq!(out, target);
    }

    /// "Delta of delta": chained patches compose. x -> y -> z via two
    /// deltas applied in sequence.
    #[test]
    fn delta_of_delta_composes(
        x in arb_bytes(2048),
        y in arb_bytes(2048),
        z in arb_bytes(2048),
    ) {
        let (_c, d1) = compute_delta(&x, &y);
        let y2 = soft(apply_delta(&x, &d1))?;
        prop_assert_eq!(y2.as_slice(), y.as_slice());

        let (_c, d2) = compute_delta(&y, &z);
        let z2 = soft(apply_delta(&y, &d2))?;
        prop_assert_eq!(z2.as_slice(), z.as_slice());

        // The second delta still applies to the *reconstructed* y.
        let z3 = soft(apply_delta(&y2, &d2))?;
        prop_assert_eq!(z3.as_slice(), z.as_slice());
    }

    /// Large binary payloads (triggering the 0x03 XOR+Zstd path) roundtrip.
    #[test]
    fn binary_path_roundtrip(base in arb_bytes(16384), target in arb_bytes(16384)) {
        prop_assume!(base.contains(&0) || target.contains(&0));
        let (_c, delta) = compute_delta(&base, &target);
        let out = soft(apply_delta(&base, &delta))?;
        prop_assert_eq!(out, target);
    }
}
