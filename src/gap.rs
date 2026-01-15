//! Gap and rift analysis algorithms.
//!
//! This module implements the analysis of discontinuities (rifts) between
//! audio data blocks. Rifts can occur due to:
//!
//! - Dropped samples (missing data)
//! - Stuttering (duplicated samples)
//! - Garbage (corrupted data between valid regions)
//!
//! The algorithms here determine the type of rift and how to repair it.

#![allow(dead_code)]

use crate::constants::MIN_WORDS_RIFT;

/// Result of rift analysis between two blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiftType {
    /// Blocks align perfectly
    NoRift,
    /// Samples were dropped from block A (need to insert from B)
    DroppedFromA(i64),
    /// Stuttering in block A (duplicate samples to remove)
    StutterInA(i64),
    /// Samples were dropped from block B (need to insert from A)
    DroppedFromB(i64),
    /// Stuttering in block B
    StutterInB(i64),
    /// Garbage section with resync point
    Garbage {
        /// Position where garbage ends in A
        sync_a: i64,
        /// Position where garbage ends in B
        sync_b: i64,
    },
    /// Unable to determine rift type
    Unknown,
}

/// Compare blocks forward until samples differ.
///
/// Starting from `pos_a` in block A and `pos_b` in block B,
/// counts how many consecutive samples match.
///
/// Returns the number of matching samples.
pub fn overlap_forward(
    block_a: &[i16],
    offset_a: i64,
    pos_a: i64,
    block_b: &[i16],
    offset_b: i64,
    pos_b: i64,
) -> i64 {
    let mut count = 0i64;
    let mut idx_a = (pos_a - offset_a) as usize;
    let mut idx_b = (pos_b - offset_b) as usize;

    while idx_a < block_a.len() && idx_b < block_b.len() {
        if block_a[idx_a] != block_b[idx_b] {
            break;
        }
        count += 1;
        idx_a += 1;
        idx_b += 1;
    }

    count
}

/// Compare blocks backward until samples differ.
///
/// Starting from `pos_a` in block A and `pos_b` in block B,
/// counts how many consecutive samples match going backward.
///
/// Returns the number of matching samples.
pub fn overlap_backward(
    block_a: &[i16],
    offset_a: i64,
    pos_a: i64,
    block_b: &[i16],
    offset_b: i64,
    pos_b: i64,
) -> i64 {
    let mut count = 0i64;
    let mut idx_a = (pos_a - offset_a) as isize;
    let mut idx_b = (pos_b - offset_b) as isize;

    while idx_a >= 0 && idx_b >= 0 {
        if block_a[idx_a as usize] != block_b[idx_b as usize] {
            break;
        }
        count += 1;
        idx_a -= 1;
        idx_b -= 1;
    }

    count
}

/// Distinguish between stuttering (duplicate samples) and a gap (missing samples).
///
/// When we find a rift, we need to determine if:
/// - Block A has extra samples (stuttering) that should be removed
/// - Block B has extra samples (stuttering) that should be removed
/// - One block is missing samples that the other has
///
/// Returns (match_in_a, match_in_b):
/// - Positive match_in_a: A has extra samples that don't appear in B
/// - Negative match_in_a: A is missing samples that B has
/// - Same interpretation for match_in_b
pub fn stutter_or_gap(
    block_a: &[i16],
    offset_a: i64,
    pos_a: i64,
    block_b: &[i16],
    offset_b: i64,
    pos_b: i64,
    max_search: i64,
) -> (i64, i64) {
    let mut match_a = 0i64;
    let mut match_b = 0i64;

    // Search forward in A for the value at pos_b in B
    let target_b = {
        let idx = (pos_b - offset_b) as usize;
        if idx < block_b.len() {
            Some(block_b[idx])
        } else {
            None
        }
    };

    if let Some(target) = target_b {
        let base_idx = (pos_a - offset_a) as usize;
        for i in 0..max_search as usize {
            let idx = base_idx + i;
            if idx >= block_a.len() {
                break;
            }
            if block_a[idx] == target {
                // Found match - verify it's a real sync point
                let verify_count = overlap_forward(
                    block_a,
                    offset_a,
                    pos_a + i as i64,
                    block_b,
                    offset_b,
                    pos_b,
                );
                if verify_count >= MIN_WORDS_RIFT {
                    match_a = i as i64;
                    break;
                }
            }
        }
    }

    // Search forward in B for the value at pos_a in A
    let target_a = {
        let idx = (pos_a - offset_a) as usize;
        if idx < block_a.len() {
            Some(block_a[idx])
        } else {
            None
        }
    };

    if let Some(target) = target_a {
        let base_idx = (pos_b - offset_b) as usize;
        for i in 0..max_search as usize {
            let idx = base_idx + i;
            if idx >= block_b.len() {
                break;
            }
            if block_b[idx] == target {
                // Found match - verify it's a real sync point
                let verify_count = overlap_forward(
                    block_a,
                    offset_a,
                    pos_a,
                    block_b,
                    offset_b,
                    pos_b + i as i64,
                );
                if verify_count >= MIN_WORDS_RIFT {
                    match_b = i as i64;
                    break;
                }
            }
        }
    }

    (match_a, match_b)
}

/// Analyze a forward (trailing) rift.
///
/// Called when we've found matching data up to a point, then a mismatch.
/// Determines what kind of rift it is and where it ends.
pub fn analyze_rift_forward(
    block_a: &[i16],
    offset_a: i64,
    end_a: i64,
    block_b: &[i16],
    offset_b: i64,
    end_b: i64,
    max_search: i64,
) -> RiftType {
    // Both blocks should have data at the mismatch point
    let idx_a = (end_a - offset_a) as usize;
    let idx_b = (end_b - offset_b) as usize;

    if idx_a >= block_a.len() || idx_b >= block_b.len() {
        return RiftType::NoRift;
    }

    let (match_a, match_b) = stutter_or_gap(
        block_a, offset_a, end_a, block_b, offset_b, end_b, max_search,
    );

    match (match_a, match_b) {
        (0, 0) => RiftType::Unknown,
        (a, 0) if a > 0 => RiftType::StutterInA(a),
        (0, b) if b > 0 => RiftType::StutterInB(b),
        (a, b) if a > 0 && b > 0 => {
            // Both found matches - this is likely garbage
            RiftType::Garbage {
                sync_a: end_a + a,
                sync_b: end_b + b,
            }
        }
        _ => RiftType::Unknown,
    }
}

/// Analyze a backward (leading) rift.
///
/// Called when we've found matching data from a point going backward,
/// then hit a mismatch. Determines what kind of rift it is.
pub fn analyze_rift_backward(
    block_a: &[i16],
    offset_a: i64,
    start_a: i64,
    block_b: &[i16],
    offset_b: i64,
    start_b: i64,
    max_search: i64,
) -> RiftType {
    // Similar to forward, but searching backward
    let idx_a = (start_a - offset_a) as isize;
    let idx_b = (start_b - offset_b) as isize;

    if idx_a < 0 || idx_b < 0 {
        return RiftType::NoRift;
    }

    // Search backward for resync
    let mut match_a = 0i64;
    let mut match_b = 0i64;

    // Get target values
    let target_b = block_b.get(idx_b as usize).copied();
    let target_a = block_a.get(idx_a as usize).copied();

    // Search backward in A for target_b
    if let Some(target) = target_b {
        for i in 0..max_search {
            let search_idx = idx_a - i as isize;
            if search_idx < 0 {
                break;
            }
            if block_a[search_idx as usize] == target {
                let verify =
                    overlap_backward(block_a, offset_a, start_a - i, block_b, offset_b, start_b);
                if verify >= MIN_WORDS_RIFT {
                    match_a = i;
                    break;
                }
            }
        }
    }

    // Search backward in B for target_a
    if let Some(target) = target_a {
        for i in 0..max_search {
            let search_idx = idx_b - i as isize;
            if search_idx < 0 {
                break;
            }
            if block_b[search_idx as usize] == target {
                let verify =
                    overlap_backward(block_a, offset_a, start_a, block_b, offset_b, start_b - i);
                if verify >= MIN_WORDS_RIFT {
                    match_b = i;
                    break;
                }
            }
        }
    }

    match (match_a, match_b) {
        (0, 0) => RiftType::Unknown,
        (a, 0) if a > 0 => RiftType::DroppedFromA(a),
        (0, b) if b > 0 => RiftType::DroppedFromB(b),
        (a, b) if a > 0 && b > 0 => RiftType::Garbage {
            sync_a: start_a - a,
            sync_b: start_b - b,
        },
        _ => RiftType::Unknown,
    }
}

/// Check if a region is silent (all zeros or near-zero values).
pub fn is_silent(data: &[i16], threshold: i16) -> bool {
    data.iter().all(|&s| s.abs() <= threshold)
}

/// Find the boundary between silent and non-silent regions.
///
/// Searches forward from `start` for the first non-silent sample.
/// Returns the position of the boundary, or None if all silent.
pub fn find_silence_boundary(data: &[i16], offset: i64, start: i64, threshold: i16) -> Option<i64> {
    let start_idx = (start - offset) as usize;
    if start_idx >= data.len() {
        return None;
    }

    for (i, &sample) in data[start_idx..].iter().enumerate() {
        if sample.abs() > threshold {
            return Some(start + i as i64);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlap_forward() {
        let a = vec![1i16, 2, 3, 4, 5, 6];
        let b = vec![1i16, 2, 3, 9, 9, 9];

        let count = overlap_forward(&a, 0, 0, &b, 0, 0);
        assert_eq!(count, 3);
    }

    #[test]
    fn test_overlap_backward() {
        let a = vec![9i16, 9, 1, 2, 3, 4];
        let b = vec![9i16, 9, 1, 2, 3, 4];

        // All 6 elements match when comparing backward from position 5
        let count = overlap_backward(&a, 0, 5, &b, 0, 5);
        assert_eq!(count, 6);

        // Test with partial match
        let c = vec![0i16, 0, 1, 2, 3, 4];
        let count2 = overlap_backward(&a, 0, 5, &c, 0, 5);
        assert_eq!(count2, 4); // Only 1, 2, 3, 4 match
    }

    #[test]
    fn test_is_silent() {
        let silent = vec![0i16, 1, -1, 0, 2, -2];
        let noisy = vec![0i16, 1, 100, 0, 2, -2];

        assert!(is_silent(&silent, 5));
        assert!(!is_silent(&noisy, 5));
    }

    #[test]
    fn test_find_silence_boundary() {
        let data = vec![0i16, 0, 0, 100, 200, 0];

        let boundary = find_silence_boundary(&data, 0, 0, 5);
        assert_eq!(boundary, Some(3));
    }
}
