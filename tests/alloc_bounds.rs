// Allocation-bound gate for `apply_delta`: a counting global allocator
// proves the apply path never over-allocates beyond the declared output
// size, regardless of instruction count.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Allocation counter tests for `apply_delta` / `apply_delta_lenient`.
//!
//! REQ-DK-100 requires that applying a delta "never panics or
//! over-allocates beyond declared limits". The declared limit is the
//! `target_len` header: the 0x02 apply path does exactly one
//! `Vec::with_capacity(target_len)` and fills it, so the allocation count
//! must be a small constant *independent of the instruction count*. This
//! binary is the verification (a counting allocator cannot lie about code
//! reading): a 10,000-instruction delta and a 40,000-instruction delta
//! over the same output size must both stay inside the same tight budget.
//!
//! No zero-allocation claim is made for this crate by design:
//! `compute_delta`/`apply_delta` return owned `Vec<u8>`s and must
//! allocate their outputs; the claim under test is the *bound*.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use delta_kit::{apply_delta, apply_delta_lenient};

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations() -> usize {
    ALLOCATIONS.load(Ordering::Relaxed)
}

/// Deterministic ASCII-ish base (no zero bytes, so the rolling strategy
/// applies).
fn base_data(size: usize) -> Vec<u8> {
    (0..size)
        .map(|i| b"abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"[i % 62])
        .collect()
}

/// Hand-assemble a `0x02` delta: `instrs` Copy records covering
/// `target_len` bytes of `base` (one 256-byte Copy per 256 output bytes).
fn copy_only_delta(base_len: usize, target_len: usize, instrs: usize) -> Vec<u8> {
    assert_eq!(instrs * 256, target_len, "each Copy covers 256 bytes");
    let mut delta = Vec::with_capacity(13 + instrs * 13);
    delta.push(0x02); // OP_INSTRUCTIONS
    delta.extend_from_slice(&(target_len as u64).to_le_bytes());
    delta.extend_from_slice(&(instrs as u32).to_le_bytes());
    for i in 0..instrs {
        let offset = (i * 256) % base_len.min(i * 256 + 1).max(256);
        let offset = if offset + 256 <= base_len { offset } else { 0 };
        delta.push(0x01); // INSTR_COPY
        delta.extend_from_slice(&(offset as u64).to_le_bytes());
        delta.extend_from_slice(&256u32.to_le_bytes());
    }
    delta
}

/// One sequential test: the allocation counter is process-global, so
/// parallel test threads would pollute each other's counts.
#[test]
fn apply_delta_allocation_bounds() {
    let base = base_data(1024 * 1024);

    // --- 10,000 Copy instructions over a 2.5 MiB output. ---
    let delta_10k = copy_only_delta(base.len(), 2_560_000, 10_000);
    let before = allocations();
    let out = apply_delta(&base, &delta_10k).expect("delta must apply");
    let allocs_10k = allocations() - before;
    assert_eq!(out.len(), 2_560_000);
    assert!(
        allocs_10k <= 8,
        "apply of a 10k-instruction delta must allocate O(1) buffers \
         (got {allocs_10k}; the target_len header is the declared bound)"
    );

    // --- 40,000 instructions, same output budget class: the count must
    // not scale with the instruction count. ---
    let delta_40k = copy_only_delta(base.len(), 10_240_000, 40_000);
    let before = allocations();
    let out = apply_delta(&base, &delta_40k).expect("delta must apply");
    let allocs_40k = allocations() - before;
    assert_eq!(out.len(), 10_240_000);
    assert!(
        allocs_40k <= 8,
        "apply of a 40k-instruction delta must stay in the same O(1) budget \
         (got {allocs_40k})"
    );

    // --- Lenient path: same bound. ---
    let before = allocations();
    let out = apply_delta_lenient(&base, &delta_10k);
    let allocs_lenient = allocations() - before;
    assert_eq!(out.len(), 2_560_000);
    assert!(
        allocs_lenient <= 8,
        "lenient apply must respect the same O(1) buffer bound (got {allocs_lenient})"
    );

    // --- Counter sanity guard: building the deltas allocates plenty.
    // If this ever fails, the bounds above prove nothing (the counter
    // would be broken, not the apply path miraculously free). ---
    let before = allocations();
    drop(copy_only_delta(base.len(), 2_560_000, 10_000));
    assert!(
        allocations() > before,
        "delta assembly must allocate — counter sanity check"
    );
}
