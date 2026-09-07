//! Hot-path benchmarks: rolling-delta compute on representative inputs.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use delta_kit::compute_delta;

/// Simulates a text file edit: 64 KiB base, ~1 KiB changed in the middle.
fn bench_text_edit(c: &mut Criterion) {
    let base: Vec<u8> = (0..65_536)
        .map(|i| b"The quick brown fox jumps over the lazy dog. "[i % 45])
        .collect();
    let mut target = base.clone();
    target[30_000..31_024].copy_from_slice(&vec![b'X'; 1_024]);

    c.bench_function("delta_text_edit_64k", |b| {
        b.iter(|| compute_delta(black_box(&base), black_box(&target)))
    });
}

/// Binary-ish input (worst case for the prefix/suffix fast path).
fn bench_binary_similar(c: &mut Criterion) {
    let base: Vec<u8> = (0..65_536u64)
        .map(|i| (i.wrapping_mul(31) >> 3) as u8)
        .collect();
    let mut target = base.clone();
    for b in &mut target[10_000..11_000] {
        *b = b.wrapping_add(1);
    }

    c.bench_function("delta_binary_edit_64k", |b| {
        b.iter(|| compute_delta(black_box(&base), black_box(&target)))
    });
}

/// Append-only growth (prefix fast-path).
fn bench_append(c: &mut Criterion) {
    let base: Vec<u8> = (0..65_536).map(|i| (i % 251) as u8).collect();
    let mut target = base.clone();
    target.extend_from_slice(&vec![b'E'; 2_048]);

    c.bench_function("delta_append_64k", |b| {
        b.iter(|| compute_delta(black_box(&base), black_box(&target)))
    });
}

criterion_group!(benches, bench_text_edit, bench_binary_similar, bench_append);
criterion_main!(benches);
