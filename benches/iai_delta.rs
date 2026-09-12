// iai-callgrind benchmarks run once under Valgrind on fixed inputs; the
// harness measures instruction counts, so there is no "expected failure"
// recovery path — a panic aborts the run visibly, which is what we want.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Deterministic regression gate for the encode/apply hot loops.
//!
//! Unlike criterion (wall-clock trend — `benches/delta_bench.rs`),
//! iai-callgrind counts CPU instructions under Valgrind and is
//! reproducible for a given binary — fit for a CI gate.
//!
//! Paths pinned (128 KiB similar inputs, ASCII so the binary sniff does
//! not divert to the XOR strategy):
//!
//! - `encode_rolling` — `compute_delta` over two similar 128 KiB inputs:
//!   the Rabin rolling-hash scan (the 0x02 hot loop)
//! - `apply_rolling` — `apply_delta` of the delta computed in setup: the
//!   Copy/Insert reconstruction loop

use delta_kit::{apply_delta, compute_delta};
use iai_callgrind::{library_benchmark, library_benchmark_group, main};

const SIZE: usize = 128 * 1024;

/// Length-derived modulo keeps the index in bounds even if the phrase is
/// edited (a hard-coded `i % 56` here once panicked: the phrase is 55 B).
const PHRASE: &[u8] = b"The quick brown fox jumps over the lazy dog. 0123456789";

/// Similar 128 KiB pair: a deterministic ASCII-ish base with a 4 KiB
/// scramble in the middle, so the rolling-hash strategy fires and the
/// instruction stream is non-trivial.
fn similar_pair() -> (Vec<u8>, Vec<u8>) {
    let base: Vec<u8> = (0..SIZE).map(|i| PHRASE[i % PHRASE.len()]).collect();
    let mut target = base.clone();
    for b in &mut target[SIZE / 2..SIZE / 2 + 4096] {
        *b = b.wrapping_add(17);
    }
    (base, target)
}

type Env = (Vec<u8>, Vec<u8>, Vec<u8>);

fn setup_encode() -> Env {
    let (base, target) = similar_pair();
    (base, target, Vec::new())
}

fn setup_apply() -> Env {
    let (base, target) = similar_pair();
    let (_copy, delta) = compute_delta(&base, &target);
    assert_eq!(delta[0], 0x02, "setup must produce a rolling-hash delta");
    (base, target, delta)
}

// The 0x02 encode hot loop: block-index build + rolling-window scan over
// 128 KiB similar inputs.
#[library_benchmark]
#[bench::rolling_128k(setup = setup_encode)]
fn encode_rolling(env: Env) -> usize {
    let (base, target, _delta) = env;
    let (_copy, delta) = compute_delta(&base, &target);
    delta.len()
}

// The 0x02 apply hot loop: Copy/Insert reconstruction of 128 KiB.
#[library_benchmark]
#[bench::rolling_128k(setup = setup_apply)]
fn apply_rolling(env: Env) -> usize {
    let (base, _target, delta) = env;
    apply_delta(&base, &delta)
        .expect("computed delta must apply")
        .len()
}

library_benchmark_group!(
    name = iai_delta_hot_path;
    benchmarks = encode_rolling, apply_rolling
);

main!(library_benchmark_groups = iai_delta_hot_path);
