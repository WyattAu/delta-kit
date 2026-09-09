// SPDX-License-Identifier: MIT OR Apache-2.0
//! `delta-kit` — a binary-safe delta codec.
//!
//! `compute_delta` produces a compact delta between a base and a target
//! byte string; `apply_delta` reconstructs the target from the base and
//! the delta. The wire format is byte-compatible with the
//! `suture-protocol` crate this codec was extracted from.
//!
//! # Strategies
//!
//! `compute_delta` picks the smallest applicable strategy:
//!
//! 1. **Binary (XOR + Zstd)** — if either input looks binary (contains a
//!    zero byte in the first 8 KiB). Emits opcode `0x03` when the XOR
//!    stream compresses smaller than the target; requires the `zstd`
//!    feature.
//! 2. **Rabin rolling hash** — when both inputs are at least
//!    [`BLOCK_SIZE`] (4 KiB). Splits the base into fixed blocks, indexes
//!    them by a Rabin rolling hash (base 257 over the Mersenne prime
//!    2^61 − 1) plus an FNV-1a-style strong hash, then scans the target
//!    with a rolling window emitting `Copy`/`Insert` instructions
//!    (opcode `0x02`). Emits only if smaller than the target.
//! 3. **Prefix/suffix trim** — small inputs. Finds the common prefix and
//!    suffix and stores only the changed middle (opcode `0x01`).
//! 4. **Full content** — fallback storing the whole target (opcode
//!    `0x00`).
//!
//! # Wire format
//!
//! All integers are little-endian. The first byte selects the encoding:
//!
//! ```text
//! 0x00 — full content
//!   [1..]              entire target
//!
//! 0x01 — prefix/suffix patch
//!   [1..9)    u64     prefix_len   (bytes reused from the base head)
//!   [9..17)   u64     suffix_len   (bytes reused from the base tail)
//!   [17..25)  u64     target_len
//!   [25..]            changed middle bytes of the target
//!
//! 0x02 — rolling-hash instruction stream
//!   [1..9)    u64     target_len
//!   [9..13)   u32     instruction_count
//!   then instruction_count records:
//!     0x01 Copy   [1..9)   u64  base_offset
//!                 [9..13)  u32  length            (13 bytes total)
//!     0x02 Insert [1..5)   u32  length
//!                 [5..5+n) bytes                    (5+n bytes total)
//!
//! 0x03 — binary XOR + Zstd   (requires the `zstd` feature)
//!   [1..9)    u64     target_len
//!   [9..25)           blake3(base)[..16]
//!   [25..41)          blake3(target)[..16]
//!   [41..]            Zstd frame over xor(base, target)
//! ```
//!
//! # Hardened decoding
//!
//! The origin implementation silently repaired malformed deltas
//! (identity-decoding truncated headers, skipping out-of-range copies,
//! returning an empty vector on checksum mismatch). This codec returns
//! [`DeltaError`] for those cases instead. Well-formed deltas — including
//! everything `compute_delta` produces — decode identically, and the
//! opcode stream is byte-for-byte the same. Consumers that must preserve
//! the origin's observable lenient behavior can delegate to
//! [`apply_delta_lenient`] instead.
//!
//! # Example
//!
//! ```
//! let base = b"Hello, World!";
//! let target = b"Hello, Rust!";
//! let (_base_copy, delta) = delta_kit::compute_delta(base, target);
//! assert_eq!(
//!     delta_kit::apply_delta(base, &delta).expect("compute_delta output always applies"),
//!     target.to_vec()
//! );
//! ```

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))] // tests assert invariants directly
#![deny(missing_docs)]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod delta;

#[cfg(feature = "zstd")]
pub use delta::compute_binary_delta;
pub use delta::BLOCK_SIZE;
pub use delta::{apply_delta, apply_delta_lenient, compute_delta};
pub use delta::{DeltaError, MismatchSide};
