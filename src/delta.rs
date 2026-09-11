// SPDX-License-Identifier: MIT OR Apache-2.0
//! Delta codec implementation. See the crate root for the wire format.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use hashbrown::HashMap;
use thiserror::Error;

/// Block size used by the Rabin rolling-hash strategy (4 KiB).
pub const BLOCK_SIZE: usize = 4096;

const RABIN_BASE: u64 = 257;
const MERSENNE61: u64 = (1u64 << 61) - 1;

#[cfg(feature = "zstd")]
const BINARY_CHECK_WINDOW: usize = 8192;

const OP_FULL: u8 = 0x00;
const OP_PREFIX_SUFFIX: u8 = 0x01;
const OP_INSTRUCTIONS: u8 = 0x02;
const OP_BINARY_XOR: u8 = 0x03;

const INSTR_COPY: u8 = 0x01;
const INSTR_INSERT: u8 = 0x02;

/// Which declared value or checksum failed to verify while applying a delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchSide {
    /// The declared target length did not match the reconstruction.
    Target,
    /// The base checksum did not match.
    Base,
    /// The reconstructed target checksum did not match.
    Result,
}

impl core::fmt::Display for MismatchSide {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            MismatchSide::Target => "target",
            MismatchSide::Base => "base",
            MismatchSide::Result => "result",
        })
    }
}

/// Errors produced while applying a delta.
///
/// `compute_delta` is total and cannot fail; only `apply_delta` validates.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DeltaError {
    /// The delta was empty (no opcode byte).
    #[error("delta is empty")]
    Empty,

    /// The top-level opcode is not one of `0x00..=0x03`.
    #[error("invalid delta opcode: {0:#04x}")]
    InvalidOpcode(u8),

    /// A header or record ended before the format requires.
    #[error("truncated delta (opcode {opcode:#04x}): needed {needed} bytes, got {got}")]
    Truncated {
        /// The encoding opcode being decoded.
        opcode: u8,
        /// Minimum bytes the format requires at this point.
        needed: usize,
        /// Bytes actually available.
        got: usize,
    },

    /// An instruction byte inside a `0x02` stream is not `0x01`/`0x02`.
    #[error("invalid instruction opcode: {0:#04x}")]
    InvalidInstruction(u8),

    /// A `Copy` instruction referenced bytes outside the base.
    #[error("copy out of range: base_offset {base_offset} + length {length} exceeds base length {base_len}")]
    CopyOutOfRange {
        /// Declared start offset into the base.
        base_offset: u64,
        /// Declared copy length.
        length: u32,
        /// Actual base length.
        base_len: usize,
    },

    /// An `Insert` instruction declared more data than the delta holds.
    #[error("insert out of range: declared {declared} bytes, {remaining} available")]
    InsertOutOfRange {
        /// Declared insert length.
        declared: u32,
        /// Bytes remaining in the delta.
        remaining: usize,
    },

    /// The reconstructed output did not match the declared target length.
    #[error("target length mismatch ({side}): declared {declared}, reconstructed {reconstructed}")]
    TargetLengthMismatch {
        /// Which declared length was contradicted.
        side: MismatchSide,
        /// The length declared in the delta.
        declared: usize,
        /// The length actually reconstructed.
        reconstructed: usize,
    },

    /// A `blake3` prefix checksum of a `0x03` binary delta did not match.
    #[error("checksum mismatch: {side} checksum does not match")]
    ChecksumMismatch {
        /// Which half failed (base or reconstructed target).
        side: MismatchSide,
    },

    /// The embedded Zstd frame failed to decompress.
    #[error("decompression error: {0}")]
    Decompression(String),

    /// A `0x03` binary delta was received by a build compiled without the
    /// `zstd` feature; it cannot be decoded.
    #[error("binary (0x03) delta requires the \"zstd\" feature")]
    ZstdDisabled,
}

#[inline]
fn mersenne_reduce(x: u128) -> u64 {
    let mut r = (x & u128::from(MERSENNE61)) + (x >> 61);
    if r >= u128::from(MERSENNE61) {
        r -= u128::from(MERSENNE61);
    }
    r as u64
}

#[inline]
fn mod_sub(a: u64, b: u64) -> u64 {
    mersenne_reduce(u128::from(a) + u128::from(MERSENNE61) - u128::from(b))
}

fn mod_pow(mut base: u64, mut exp: usize) -> u64 {
    let mut result: u64 = 1;
    while exp > 0 {
        if exp & 1 == 1 {
            result = mersenne_reduce(u128::from(result) * u128::from(base));
        }
        base = mersenne_reduce(u128::from(base) * u128::from(base));
        exp >>= 1;
    }
    result
}

fn rabin_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 0;
    for &b in data {
        h = mersenne_reduce(u128::from(h) * u128::from(RABIN_BASE) + u128::from(b));
    }
    h
}

fn rabin_roll(h: u64, old_byte: u8, new_byte: u8, base_power: u64) -> u64 {
    let old_contrib = mersenne_reduce(u128::from(old_byte) * u128::from(base_power));
    let h2 = mod_sub(h, old_contrib);
    mersenne_reduce(u128::from(h2) * u128::from(RABIN_BASE) + u128::from(new_byte))
}

fn strong_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_3ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    h
}

#[cfg(feature = "zstd")]
fn is_likely_binary(data: &[u8]) -> bool {
    let window = core::cmp::min(data.len(), BINARY_CHECK_WINDOW);
    data[..window].contains(&0u8)
}

/// XOR the target against the base (base-length prefix), appending the
/// target tail beyond the base length.
#[cfg(feature = "zstd")]
fn xor_streams(base: &[u8], target: &[u8]) -> Vec<u8> {
    let min_len = base.len().min(target.len());
    let mut xor_data = Vec::with_capacity(target.len());
    for i in 0..min_len {
        xor_data.push(base[i] ^ target[i]);
    }
    if target.len() > base.len() {
        xor_data.extend_from_slice(&target[base.len()..]);
    }
    xor_data
}

/// Compute a `0x03` binary XOR+Zstd delta directly, bypassing the
/// binary-sniffing gate of [`compute_delta`].
///
/// Returns `None` when the compressed XOR stream is not smaller than the
/// target itself. Requires the `zstd` feature.
#[cfg(feature = "zstd")]
#[must_use]
pub fn compute_binary_delta(base: &[u8], target: &[u8]) -> Option<Vec<u8>> {
    let base_hash = blake3::hash(base);
    let target_hash = blake3::hash(target);

    let xor_data = xor_streams(base, target);
    let compressed = zstd::encode_all(xor_data.as_slice(), 3).ok()?;

    if compressed.len() >= target.len() {
        return None;
    }

    let mut delta = Vec::with_capacity(41 + compressed.len());
    delta.push(OP_BINARY_XOR);
    delta.extend_from_slice(&(target.len() as u64).to_le_bytes());
    delta.extend_from_slice(&base_hash.as_bytes()[..16]);
    delta.extend_from_slice(&target_hash.as_bytes()[..16]);
    delta.extend_from_slice(&compressed);

    Some(delta)
}

enum DeltaInstr {
    Copy { base_offset: u64, length: u32 },
    Insert { data: Vec<u8> },
}

fn compute_rolling_delta(base: &[u8], target: &[u8]) -> Option<Vec<u8>> {
    if base.len() < BLOCK_SIZE || target.len() < BLOCK_SIZE {
        return None;
    }

    let num_blocks = base.len() / BLOCK_SIZE;
    if num_blocks == 0 {
        return None;
    }

    let mut hash_table: HashMap<u64, Vec<(usize, u64)>> = HashMap::new();
    for i in 0..num_blocks {
        let block = &base[i * BLOCK_SIZE..(i + 1) * BLOCK_SIZE];
        let rh = rabin_hash(block);
        let sh = strong_hash(block);
        hash_table.entry(rh).or_default().push((i, sh));
    }

    let base_power = mod_pow(RABIN_BASE, BLOCK_SIZE - 1);
    let mut instructions: Vec<DeltaInstr> = Vec::new();
    let mut pending_insert_start: usize = 0;
    let mut pos: usize = 0;
    let mut prev_rabin: Option<u64> = None;

    while pos + BLOCK_SIZE <= target.len() {
        let rh = match prev_rabin {
            Some(pr) if pos > 0 => rabin_roll(
                pr,
                target[pos - 1],
                target[pos + BLOCK_SIZE - 1],
                base_power,
            ),
            _ => rabin_hash(&target[pos..pos + BLOCK_SIZE]),
        };
        prev_rabin = Some(rh);

        let mut matched = false;
        if let Some(candidates) = hash_table.get(&rh) {
            let sh = strong_hash(&target[pos..pos + BLOCK_SIZE]);
            for &(block_idx, ref_sh) in candidates {
                if sh == ref_sh {
                    let base_offset = block_idx * BLOCK_SIZE;
                    let mut match_len = BLOCK_SIZE;

                    while pos + match_len < target.len()
                        && base_offset + match_len < base.len()
                        && target[pos + match_len] == base[base_offset + match_len]
                    {
                        match_len += 1;
                    }

                    let match_len = match_len.min(u32::MAX as usize);

                    if pending_insert_start < pos {
                        instructions.push(DeltaInstr::Insert {
                            data: target[pending_insert_start..pos].to_vec(),
                        });
                    }

                    instructions.push(DeltaInstr::Copy {
                        base_offset: base_offset as u64,
                        length: match_len as u32,
                    });

                    pos += match_len;
                    pending_insert_start = pos;
                    prev_rabin = None;
                    matched = true;
                    break;
                }
            }
        }

        if !matched {
            pos += 1;
        }
    }

    if pending_insert_start < target.len() {
        instructions.push(DeltaInstr::Insert {
            data: target[pending_insert_start..].to_vec(),
        });
    }

    let mut delta = Vec::new();
    delta.push(OP_INSTRUCTIONS);
    delta.extend_from_slice(&(target.len() as u64).to_le_bytes());
    delta.extend_from_slice(&(instructions.len() as u32).to_le_bytes());

    for instr in &instructions {
        match instr {
            DeltaInstr::Copy {
                base_offset,
                length,
            } => {
                delta.push(INSTR_COPY);
                delta.extend_from_slice(&base_offset.to_le_bytes());
                delta.extend_from_slice(&length.to_le_bytes());
            }
            DeltaInstr::Insert { data } => {
                delta.push(INSTR_INSERT);
                delta.extend_from_slice(&(data.len() as u32).to_le_bytes());
                delta.extend_from_slice(data);
            }
        }
    }

    if delta.len() < target.len() {
        Some(delta)
    } else {
        None
    }
}

/// Compute a delta transforming `base` into `target`.
///
/// Returns `(base_copy, delta)`. The first element is simply a copy of
/// `base`, kept for API compatibility with the origin `suture-protocol`
/// signature. The delta always reconstructs `target` via [`apply_delta`]
/// and uses the smallest applicable encoding (see the crate docs).
#[must_use]
pub fn compute_delta(base: &[u8], target: &[u8]) -> (Vec<u8>, Vec<u8>) {
    #[cfg(feature = "zstd")]
    if is_likely_binary(base) || is_likely_binary(target) {
        if let Some(delta) = compute_binary_delta(base, target) {
            return (base.to_vec(), delta);
        }
    }

    if base.len() >= BLOCK_SIZE && target.len() >= BLOCK_SIZE {
        if let Some(delta) = compute_rolling_delta(base, target) {
            return (base.to_vec(), delta);
        }
        let mut full = vec![OP_FULL];
        full.extend_from_slice(target);
        return (base.to_vec(), full);
    }

    let prefix_len = base
        .iter()
        .zip(target.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let max_suffix_base = base.len().saturating_sub(prefix_len);
    let max_suffix_target = target.len().saturating_sub(prefix_len);
    let suffix_len = base[prefix_len..]
        .iter()
        .rev()
        .zip(target[prefix_len..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(max_suffix_base)
        .min(max_suffix_target);

    let changed_start = prefix_len;
    let changed_end_target = target.len().saturating_sub(suffix_len);
    let changed = &target[changed_start..changed_end_target];

    if changed.len() < target.len() {
        let mut delta = Vec::new();
        delta.push(OP_PREFIX_SUFFIX);
        delta.extend_from_slice(&(prefix_len as u64).to_le_bytes());
        delta.extend_from_slice(&(suffix_len as u64).to_le_bytes());
        delta.extend_from_slice(&(target.len() as u64).to_le_bytes());
        delta.extend_from_slice(changed);
        (base.to_vec(), delta)
    } else {
        let mut full = vec![OP_FULL];
        full.extend_from_slice(target);
        (base.to_vec(), full)
    }
}

/// Read a little-endian `u64` at `at`, requiring `at + 8 <= delta.len()`.
fn read_u64_le(delta: &[u8], at: usize, opcode: u8) -> Result<u64, DeltaError> {
    let end = at
        .checked_add(8)
        .filter(|&e| e <= delta.len())
        .ok_or(DeltaError::Truncated {
            opcode,
            needed: at + 8,
            got: delta.len(),
        })?;
    // INVARIANT (documented-infallible): `end - at == 8` by construction.
    #[allow(clippy::expect_used)]
    let bytes =
        <[u8; 8]>::try_from(&delta[at..end]).expect("slice length is exactly 8 by construction");
    Ok(u64::from_le_bytes(bytes))
}

/// Read a little-endian `u32` at `at`, requiring `at + 4 <= delta.len()`.
fn read_u32_le(delta: &[u8], at: usize, opcode: u8) -> Result<u32, DeltaError> {
    let end = at
        .checked_add(4)
        .filter(|&e| e <= delta.len())
        .ok_or(DeltaError::Truncated {
            opcode,
            needed: at + 4,
            got: delta.len(),
        })?;
    // INVARIANT (documented-infallible): `end - at == 4` by construction.
    #[allow(clippy::expect_used)]
    let bytes =
        <[u8; 4]>::try_from(&delta[at..end]).expect("slice length is exactly 4 by construction");
    Ok(u32::from_le_bytes(bytes))
}

fn length_mismatch(side: MismatchSide, declared: u64, reconstructed: usize) -> DeltaError {
    DeltaError::TargetLengthMismatch {
        side,
        declared: usize::try_from(declared).unwrap_or(usize::MAX),
        reconstructed,
    }
}

/// Apply `delta` to `base`, reconstructing the target.
///
/// This validates the delta strictly: truncated headers, unknown
/// opcodes/instructions, out-of-range copies, inserts past the end of the
/// delta, and length/checksum contradictions all return
/// [`DeltaError`]. Deltas produced by [`compute_delta`] always
/// round-trip.
pub fn apply_delta(base: &[u8], delta: &[u8]) -> Result<Vec<u8>, DeltaError> {
    let Some((&opcode, rest)) = delta.split_first() else {
        return Err(DeltaError::Empty);
    };

    match opcode {
        OP_FULL => Ok(rest.to_vec()),

        OP_PREFIX_SUFFIX => {
            if delta.len() < 25 {
                return Err(DeltaError::Truncated {
                    opcode,
                    needed: 25,
                    got: delta.len(),
                });
            }
            let prefix_len = read_u64_le(delta, 1, opcode)? as usize;
            let suffix_len = read_u64_le(delta, 9, opcode)? as usize;
            let total_len = read_u64_le(delta, 17, opcode)?;
            let changed = &delta[25..];

            let prefix_take = prefix_len.min(base.len());
            let mut result = Vec::with_capacity(total_len.min(usize::MAX as u64) as usize);
            result.extend_from_slice(&base[..prefix_take]);
            result.extend_from_slice(changed);
            result.extend_from_slice(&base[base.len().saturating_sub(suffix_len)..]);

            if result.len() as u64 != total_len {
                return Err(length_mismatch(
                    MismatchSide::Target,
                    total_len,
                    result.len(),
                ));
            }
            Ok(result)
        }

        OP_INSTRUCTIONS => {
            if delta.len() < 13 {
                return Err(DeltaError::Truncated {
                    opcode,
                    needed: 13,
                    got: delta.len(),
                });
            }
            let target_len = read_u64_le(delta, 1, opcode)?;
            let num_instr = read_u32_le(delta, 9, opcode)?;
            let mut result: Vec<u8> =
                Vec::with_capacity(target_len.min(usize::MAX as u64) as usize);
            let mut offset = 13usize;

            for _ in 0..num_instr {
                let Some(&instr) = delta.get(offset) else {
                    return Err(DeltaError::Truncated {
                        opcode,
                        needed: offset + 1,
                        got: delta.len(),
                    });
                };
                match instr {
                    INSTR_COPY => {
                        if offset + 13 > delta.len() {
                            return Err(DeltaError::Truncated {
                                opcode,
                                needed: offset + 13,
                                got: delta.len(),
                            });
                        }
                        let base_offset = read_u64_le(delta, offset + 1, opcode)?;
                        let length = read_u32_le(delta, offset + 9, opcode)?;
                        let bo = usize::try_from(base_offset).map_err(|_| {
                            DeltaError::CopyOutOfRange {
                                base_offset,
                                length,
                                base_len: base.len(),
                            }
                        })?;
                        let ln =
                            usize::try_from(length).map_err(|_| DeltaError::CopyOutOfRange {
                                base_offset,
                                length,
                                base_len: base.len(),
                            })?;
                        let end = bo.saturating_add(ln);
                        if end > base.len() {
                            return Err(DeltaError::CopyOutOfRange {
                                base_offset,
                                length,
                                base_len: base.len(),
                            });
                        }
                        result.extend_from_slice(&base[bo..end]);
                        offset += 13;
                    }
                    INSTR_INSERT => {
                        if offset + 5 > delta.len() {
                            return Err(DeltaError::Truncated {
                                opcode,
                                needed: offset + 5,
                                got: delta.len(),
                            });
                        }
                        let declared_len = read_u32_le(delta, offset + 1, opcode)?;
                        let length = usize::try_from(declared_len).map_err(|_| {
                            DeltaError::InsertOutOfRange {
                                declared: declared_len,
                                remaining: delta.len() - offset - 5,
                            }
                        })?;
                        let data_end = offset.checked_add(5).and_then(|v| v.checked_add(length));
                        match data_end {
                            None => {
                                return Err(DeltaError::InsertOutOfRange {
                                    declared: declared_len,
                                    remaining: delta.len() - offset - 5,
                                })
                            }
                            Some(data_end) if data_end > delta.len() => {
                                return Err(DeltaError::InsertOutOfRange {
                                    declared: declared_len,
                                    remaining: delta.len() - offset - 5,
                                })
                            }
                            Some(data_end) => {
                                result.extend_from_slice(&delta[offset + 5..data_end]);
                                offset = data_end;
                            }
                        }
                    }
                    other => return Err(DeltaError::InvalidInstruction(other)),
                }
            }

            if result.len() as u64 != target_len {
                return Err(length_mismatch(
                    MismatchSide::Target,
                    target_len,
                    result.len(),
                ));
            }
            Ok(result)
        }

        OP_BINARY_XOR => {
            #[cfg(feature = "zstd")]
            {
                if delta.len() < 41 {
                    return Err(DeltaError::Truncated {
                        opcode,
                        needed: 41,
                        got: delta.len(),
                    });
                }
                let target_len = read_u64_le(delta, 1, opcode)?;
                let base_checksum = &delta[9..25];
                let target_checksum = &delta[25..41];
                let compressed = &delta[41..];

                let base_hash = blake3::hash(base);
                if base_hash.as_bytes()[..16] != *base_checksum {
                    return Err(DeltaError::ChecksumMismatch {
                        side: MismatchSide::Base,
                    });
                }

                let xor_data = zstd::decode_all(compressed)
                    .map_err(|e| DeltaError::Decompression(e.to_string()))?;

                let mut result = Vec::with_capacity(target_len.min(usize::MAX as u64) as usize);
                let min_len = base.len().min(xor_data.len());
                for i in 0..min_len {
                    result.push(base[i] ^ xor_data[i]);
                }
                if xor_data.len() > base.len() {
                    result.extend_from_slice(&xor_data[base.len()..]);
                }

                if result.len() as u64 != target_len {
                    return Err(length_mismatch(
                        MismatchSide::Target,
                        target_len,
                        result.len(),
                    ));
                }

                let result_hash = blake3::hash(&result);
                if result_hash.as_bytes()[..16] != *target_checksum {
                    return Err(DeltaError::ChecksumMismatch {
                        side: MismatchSide::Result,
                    });
                }

                Ok(result)
            }

            #[cfg(not(feature = "zstd"))]
            {
                let _ = rest;
                Err(DeltaError::ZstdDisabled)
            }
        }

        other => Err(DeltaError::InvalidOpcode(other)),
    }
}

/// Read a little-endian `u64` at `at`, yielding `0` when out of bounds.
/// Mirrors the origin decoder's `try_into().unwrap_or([0; 8])` fallback.
fn read_u64_or_zero(delta: &[u8], at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    if let Some(slice) = delta.get(at..at.saturating_add(8)) {
        if slice.len() == 8 {
            bytes.copy_from_slice(slice);
        }
    }
    u64::from_le_bytes(bytes)
}

/// Read a little-endian `u32` at `at`, yielding `0` when out of bounds.
fn read_u32_or_zero(delta: &[u8], at: usize) -> u32 {
    let mut bytes = [0u8; 4];
    if let Some(slice) = delta.get(at..at.saturating_add(4)) {
        if slice.len() == 4 {
            bytes.copy_from_slice(slice);
        }
    }
    u32::from_le_bytes(bytes)
}

/// Apply `delta` to `base` with the origin `suture-protocol` semantics.
///
/// This is a behavior-preserving port of the pre-extraction decoder:
/// malformed input is silently repaired instead of rejected. For
/// consumers whose public contract is the origin's infallible
/// `apply_delta(base, delta) -> Vec<u8>`, this is the drop-in delegate.
///
/// Origin-observed behavior, reproduced exactly:
///
/// - an empty delta decodes to an empty vector;
/// - an unknown top-level opcode, a truncated header, or a failed Zstd
///   frame decode passes the delta bytes through unchanged;
/// - a `0x02` stream stops at the first truncated or unknown
///   instruction and returns the partial reconstruction; out-of-range
///   `Copy` records and past-the-end `Insert` records are skipped;
/// - a `0x03` base or target checksum mismatch returns an empty vector;
/// - declared lengths are never validated (a length is only used as an
///   allocation hint, capped against attacker-controlled values).
///
/// On well-formed deltas — everything [`compute_delta`] produces — this
/// agrees byte-for-byte with [`apply_delta`]. New code should prefer
/// [`apply_delta`].
#[must_use]
pub fn apply_delta_lenient(base: &[u8], delta: &[u8]) -> Vec<u8> {
    let Some((&opcode, rest)) = delta.split_first() else {
        return Vec::new();
    };

    match opcode {
        OP_FULL => rest.to_vec(),

        OP_PREFIX_SUFFIX => {
            if delta.len() < 25 {
                return delta.to_vec();
            }
            let prefix_len = read_u64_or_zero(delta, 1) as usize;
            let suffix_len = read_u64_or_zero(delta, 9) as usize;
            let total_len = read_u64_or_zero(delta, 17) as usize;
            let changed = &delta[25..];

            let prefix_take = prefix_len.min(base.len());
            let mut result = Vec::with_capacity(
                total_len.min(prefix_take + changed.len() + base.len().min(suffix_len)),
            );
            result.extend_from_slice(&base[..prefix_take]);
            result.extend_from_slice(changed);
            result.extend_from_slice(&base[base.len().saturating_sub(suffix_len)..]);
            result
        }

        OP_INSTRUCTIONS => {
            if delta.len() < 13 {
                return delta.to_vec();
            }
            let target_len = read_u64_or_zero(delta, 1) as usize;
            let num_instr = read_u32_or_zero(delta, 9) as usize;
            let mut result = Vec::with_capacity(target_len.min(delta.len() + base.len()));
            let mut offset = 13usize;

            for _ in 0..num_instr {
                if offset >= delta.len() {
                    break;
                }
                match delta[offset] {
                    INSTR_COPY => {
                        if offset + 13 > delta.len() {
                            break;
                        }
                        let base_offset = read_u64_or_zero(delta, offset + 1) as usize;
                        let length = read_u32_or_zero(delta, offset + 9) as usize;
                        let end = base_offset.saturating_add(length);
                        if end <= base.len() {
                            result.extend_from_slice(&base[base_offset..end]);
                        }
                        offset += 13;
                    }
                    INSTR_INSERT => {
                        if offset + 5 > delta.len() {
                            break;
                        }
                        let length = read_u32_or_zero(delta, offset + 1) as usize;
                        let data_end = offset.saturating_add(5).saturating_add(length);
                        if data_end <= delta.len() {
                            result.extend_from_slice(&delta[offset + 5..data_end]);
                            offset = data_end;
                        }
                    }
                    _ => break,
                }
            }

            result
        }

        OP_BINARY_XOR => {
            #[cfg(feature = "zstd")]
            {
                if delta.len() < 41 {
                    return delta.to_vec();
                }
                let base_checksum = &delta[9..25];
                let target_checksum = &delta[25..41];
                let compressed = &delta[41..];

                let base_hash = blake3::hash(base);
                if base_hash.as_bytes()[..16] != *base_checksum {
                    return Vec::new();
                }

                let Ok(xor_data) = zstd::decode_all(compressed) else {
                    return delta.to_vec();
                };

                let mut result = Vec::with_capacity(base.len().max(xor_data.len()));
                let min_len = base.len().min(xor_data.len());
                for i in 0..min_len {
                    result.push(base[i] ^ xor_data[i]);
                }
                if xor_data.len() > base.len() {
                    result.extend_from_slice(&xor_data[base.len()..]);
                }

                let result_hash = blake3::hash(&result);
                if result_hash.as_bytes()[..16] != *target_checksum {
                    return Vec::new();
                }

                result
            }

            #[cfg(not(feature = "zstd"))]
            {
                delta.to_vec()
            }
        }

        _ => delta.to_vec(),
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    /// All fallible steps in tests use `?`; the crate keeps a strict
    /// zero-`unwrap()` policy (verified by grep).
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    // === Ported from suture-protocol ===

    #[test]
    fn test_delta_roundtrip() {
        let base = b"Hello, World!";
        let target = b"Hello, Rust!";
        let (_base_copy, delta) = compute_delta(base, target);
        let result = apply_delta(base, &delta).expect("well-formed delta must apply");
        assert_eq!(result, target);
    }

    #[test]
    fn test_delta_no_change() {
        let base = b"identical data here";
        let target = b"identical data here";
        let (_base_copy, delta) = compute_delta(base, target);
        assert!(delta.len() < target.len() + 25);
        let result = apply_delta(base, &delta).expect("well-formed delta must apply");
        assert_eq!(result, target);
    }

    #[test]
    fn test_delta_completely_different() {
        let base = b"AAAA";
        let target = b"BBBB";
        let (_base_copy, delta) = compute_delta(base, target);
        let result = apply_delta(base, &delta).expect("well-formed delta must apply");
        assert_eq!(result, target);
    }

    // === Strategy coverage ===

    #[cfg(feature = "zstd")]
    #[test]
    fn test_binary_delta_roundtrip() -> TestResult {
        // Zero bytes trip is_likely_binary -> XOR+Zstd (0x03) path.
        let base = vec![0u8; 8192];
        let mut target = vec![0u8; 8192];
        target[100] = 0xAB;
        let last = target.len() - 1;
        target[last] = 0xCD;

        let (_c, delta) = compute_delta(&base, &target);
        assert_eq!(delta[0], OP_BINARY_XOR, "binary inputs must use 0x03");
        let result = apply_delta(&base, &delta)?;
        assert_eq!(result, target);
        Ok(())
    }

    #[test]
    fn test_rolling_delta_roundtrip_and_opcode() -> TestResult {
        // Text-like (zero-free) inputs, both >= BLOCK_SIZE -> 0x02 path.
        let base: Vec<u8> = (0..3 * BLOCK_SIZE).map(|i| b'A' + (i % 26) as u8).collect();
        let mut target = base.clone();
        target.splice(100..110, b"XX".to_vec());

        let (_c, delta) = compute_delta(&base, &target);
        assert_eq!(delta[0], OP_INSTRUCTIONS, "large text inputs must use 0x02");
        assert!(delta.len() < target.len(), "rolling delta must shrink here");
        let result = apply_delta(&base, &delta)?;
        assert_eq!(result, target);
        Ok(())
    }

    #[test]
    fn test_prefix_suffix_opcode() -> TestResult {
        let base = b"Hello, World!".to_vec();
        let target = b"Hello, Rust!".to_vec();
        let (_c, delta) = compute_delta(&base, &target);
        assert_eq!(delta[0], OP_PREFIX_SUFFIX);
        assert_eq!(apply_delta(&base, &delta)?, target);
        Ok(())
    }

    #[test]
    fn test_full_opcode_fallback() -> TestResult {
        // Completely different, tiny, binary -> 0x00 full content.
        let base = vec![0u8; 4];
        let target = vec![1u8, 2, 3];
        let (_c, delta) = compute_delta(&base, &target);
        assert_eq!(delta[0], OP_FULL);
        assert_eq!(&delta[1..], &target[..]);
        assert_eq!(apply_delta(&base, &delta)?, target);
        Ok(())
    }

    // === Hardened decode behavior ===

    #[test]
    fn test_apply_empty_delta_is_error() {
        assert_eq!(apply_delta(b"abc", &[]), Err(DeltaError::Empty));
    }

    #[test]
    fn test_apply_unknown_opcode_is_error() {
        // Origin silently identity-decoded this; we reject.
        assert_eq!(
            apply_delta(b"abc", &[0x7F, 1, 2, 3]),
            Err(DeltaError::InvalidOpcode(0x7F))
        );
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn test_apply_truncated_headers_are_errors() {
        // 0x01 needs 25 bytes.
        let short01 = vec![OP_PREFIX_SUFFIX; 10];
        assert!(matches!(
            apply_delta(b"abc", &short01),
            Err(DeltaError::Truncated {
                opcode: OP_PREFIX_SUFFIX,
                ..
            })
        ));

        // 0x02 needs 13 bytes.
        let short02 = vec![OP_INSTRUCTIONS; 8];
        assert!(matches!(
            apply_delta(b"abc", &short02),
            Err(DeltaError::Truncated {
                opcode: OP_INSTRUCTIONS,
                ..
            })
        ));
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn test_apply_truncated_binary_header_is_error() {
        // 0x03 needs 41 bytes.
        let short03 = vec![OP_BINARY_XOR; 20];
        assert!(matches!(
            apply_delta(b"abc", &short03),
            Err(DeltaError::Truncated {
                opcode: OP_BINARY_XOR,
                ..
            })
        ));
    }

    #[cfg(not(feature = "zstd"))]
    #[test]
    fn test_binary_opcode_disabled_without_feature() {
        assert_eq!(
            apply_delta(b"abc", &[OP_BINARY_XOR]),
            Err(DeltaError::ZstdDisabled)
        );
    }

    #[test]
    fn test_apply_copy_out_of_range_is_error() {
        let mut d = vec![OP_INSTRUCTIONS];
        d.extend_from_slice(&5u64.to_le_bytes()); // target_len
        d.extend_from_slice(&1u32.to_le_bytes()); // one instruction
        d.push(INSTR_COPY);
        d.extend_from_slice(&1_000u64.to_le_bytes()); // base_offset beyond base
        d.extend_from_slice(&2u32.to_le_bytes()); // length
        assert!(matches!(
            apply_delta(b"abc", &d),
            Err(DeltaError::CopyOutOfRange { .. })
        ));
    }

    #[test]
    fn test_apply_insert_past_end_is_error() {
        let mut d = vec![OP_INSTRUCTIONS];
        d.extend_from_slice(&8u64.to_le_bytes()); // target_len
        d.extend_from_slice(&1u32.to_le_bytes()); // one instruction
        d.push(INSTR_INSERT);
        d.extend_from_slice(&100u32.to_le_bytes()); // claims 100 bytes, provides 0
        assert!(matches!(
            apply_delta(b"abc", &d),
            Err(DeltaError::InsertOutOfRange { .. })
        ));
    }

    #[test]
    fn test_apply_unknown_instruction_is_error() {
        let mut d = vec![OP_INSTRUCTIONS];
        d.extend_from_slice(&1u64.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.push(0x42); // not Copy/Insert
        assert_eq!(
            apply_delta(b"abc", &d),
            Err(DeltaError::InvalidInstruction(0x42))
        );
    }

    #[test]
    fn test_apply_target_length_mismatch_is_error() {
        let mut d = vec![OP_INSTRUCTIONS];
        d.extend_from_slice(&9u64.to_le_bytes()); // claims 9...
        d.extend_from_slice(&0u32.to_le_bytes()); // ...but zero instructions
        assert!(matches!(
            apply_delta(b"abc", &d),
            Err(DeltaError::TargetLengthMismatch { .. })
        ));
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn test_apply_binary_checksum_mismatch_is_error() -> TestResult {
        // Craft a valid-looking 0x03 whose base checksum won't match.
        let mut d = vec![OP_BINARY_XOR];
        d.extend_from_slice(&4u64.to_le_bytes()); // target_len
        d.extend_from_slice(&[0u8; 16]); // wrong base checksum
        d.extend_from_slice(&[0u8; 16]); // target checksum (unreached)
        let frame = zstd::encode_all(b"abcd".as_slice(), 3)?;
        d.extend_from_slice(&frame);
        assert!(matches!(
            apply_delta(b"zzzz", &d),
            Err(DeltaError::ChecksumMismatch {
                side: MismatchSide::Base
            })
        ));
        Ok(())
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn test_binary_delta_tamper_is_error() -> TestResult {
        let base = vec![0u8; 4096];
        let target = vec![7u8; 4096];
        let (_c, mut delta) = compute_delta(&base, &target);
        assert_eq!(delta[0], OP_BINARY_XOR);
        // Flip a byte in the compressed payload.
        let last = delta.len() - 1;
        delta[last] ^= 0xFF;
        assert!(apply_delta(&base, &delta).is_err());
        Ok(())
    }

    #[test]
    fn test_empty_target() -> TestResult {
        let (_c, delta) = compute_delta(b"base", b"");
        assert_eq!(delta, vec![OP_FULL]);
        assert_eq!(apply_delta(b"base", &delta)?, Vec::<u8>::new());
        Ok(())
    }

    #[test]
    fn test_identical_empty() {
        let (_c, delta) = compute_delta(b"", b"");
        // Empty target: changed (0) < target (0) is false -> full.
        assert_eq!(delta, vec![OP_FULL]);
    }

    // === Lenient (origin-compatible) decode behavior ===

    #[test]
    fn test_lenient_empty_delta_returns_empty() {
        assert_eq!(apply_delta_lenient(b"abc", &[]), Vec::<u8>::new());
    }

    #[test]
    fn test_lenient_unknown_opcode_echoes_delta() {
        assert_eq!(
            apply_delta_lenient(b"abc", &[0x7F, 1, 2, 3]),
            vec![0x7F, 1, 2, 3]
        );
    }

    #[test]
    fn test_lenient_truncated_prefix_suffix_echoes_delta() {
        let short01 = vec![OP_PREFIX_SUFFIX; 10];
        assert_eq!(apply_delta_lenient(b"abc", &short01), short01);
    }

    #[test]
    fn test_lenient_skips_out_of_range_copy() {
        let mut d = vec![OP_INSTRUCTIONS];
        d.extend_from_slice(&5u64.to_le_bytes()); // target_len
        d.extend_from_slice(&1u32.to_le_bytes()); // one instruction
        d.push(INSTR_COPY);
        d.extend_from_slice(&1_000u64.to_le_bytes()); // beyond base
        d.extend_from_slice(&2u32.to_le_bytes());
        assert_eq!(apply_delta_lenient(b"abc", &d), Vec::<u8>::new());
    }

    #[test]
    fn test_lenient_skips_insert_past_end() {
        let mut d = vec![OP_INSTRUCTIONS];
        d.extend_from_slice(&8u64.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.push(INSTR_INSERT);
        d.extend_from_slice(&100u32.to_le_bytes()); // claims 100, provides 0
        assert_eq!(apply_delta_lenient(b"abc", &d), Vec::<u8>::new());
    }

    #[test]
    fn test_lenient_returns_partial_on_unknown_instruction() {
        let mut d = vec![OP_INSTRUCTIONS];
        d.extend_from_slice(&4u64.to_le_bytes()); // target_len
        d.extend_from_slice(&2u32.to_le_bytes()); // two instructions
        d.push(INSTR_COPY);
        d.extend_from_slice(&0u64.to_le_bytes());
        d.extend_from_slice(&4u32.to_le_bytes()); // copies "abcd"
        d.push(0x42); // unknown: stop, keep partial result
        assert_eq!(apply_delta_lenient(b"abcd", &d), b"abcd".to_vec());
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn test_lenient_checksum_mismatch_returns_empty() {
        let base = vec![0u8; 4096];
        let mut target = vec![0u8; 4096];
        target[100] = 0xAB;
        let (_c, mut delta) = compute_delta(&base, &target);
        assert_eq!(delta[0], OP_BINARY_XOR);
        delta[9] ^= 0xFF; // corrupt the base checksum
        assert_eq!(apply_delta_lenient(&base, &delta), Vec::<u8>::new());
    }

    #[test]
    fn test_lenient_agrees_with_strict_on_computed_deltas() -> TestResult {
        let cases: Vec<(Vec<u8>, Vec<u8>)> = vec![
            (b"Hello, World!".to_vec(), b"Hello, Rust!".to_vec()),
            (b"keep me".to_vec(), b"keep me too".to_vec()),
            (vec![0u8; 4], vec![1, 2, 3]),
            (
                (0..3 * BLOCK_SIZE).map(|i| b'A' + (i % 26) as u8).collect(),
                {
                    let mut t = (0..3 * BLOCK_SIZE)
                        .map(|i| b'A' + (i % 26) as u8)
                        .collect::<Vec<_>>();
                    t.splice(100..110, b"XX".to_vec());
                    t
                },
            ),
        ];
        for (base, target) in &cases {
            let (_c, delta) = compute_delta(base, target);
            assert_eq!(
                apply_delta_lenient(base, &delta),
                apply_delta(base, &delta)?,
                "lenient and strict must agree on compute_delta output"
            );
        }
        Ok(())
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn test_lenient_binary_roundtrip() -> TestResult {
        let base = vec![0u8; 8192];
        let mut target = vec![0u8; 8192];
        target[100] = 0xAB;
        let last = target.len() - 1;
        target[last] = 0xCD;
        let (_c, delta) = compute_delta(&base, &target);
        assert_eq!(delta[0], OP_BINARY_XOR);
        assert_eq!(apply_delta_lenient(&base, &delta), target);
        assert_eq!(apply_delta(&base, &delta)?, target);
        Ok(())
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn test_compute_binary_delta_public_surface() {
        let data: Vec<u8> = (0..8192).map(|i| (i % 251) as u8).collect();
        let delta = compute_binary_delta(&data, &data).expect("identical inputs compress");
        assert_eq!(delta[0], OP_BINARY_XOR);
        assert!(delta.len() < 100, "identical inputs must compress tiny");
        assert_eq!(apply_delta_lenient(&data, &delta), data);
    }
}
