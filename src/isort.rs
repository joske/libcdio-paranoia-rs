//! Fast sample value lookup using hash table indexing.
//!
//! This module implements a 65536-bucket hash table for O(1) average-case
//! lookup of sample values in audio data. This is critical for efficient
//! jitter detection and pattern matching in the paranoia algorithm.
//!
//! Each bucket corresponds to one possible 16-bit sample value (-32768 to 32767).
//! Linked lists handle collisions when multiple positions have the same value.

#![allow(dead_code)]

/// Number of buckets in the hash table (one per possible 16-bit value)
const NUM_BUCKETS: usize = 65536;

/// A link in the hash table chain
#[derive(Debug, Clone, Copy)]
pub struct SortLink {
    /// Index into the vector being indexed
    pub index: usize,
    /// Next link in chain (usize::MAX = end of chain)
    pub next: usize,
}

impl SortLink {
    pub const NONE: usize = usize::MAX;
}

/// Fast sample value lookup structure.
///
/// Indexes a vector of 16-bit samples for O(1) average lookup by value.
#[derive(Debug)]
pub struct SortInfo {
    /// Reference to the vector being indexed
    vector: Vec<i16>,
    /// Absolute position offset for the vector
    pub abs_offset: i64,
    /// Head index for each bucket (65536 buckets)
    heads: Vec<usize>,
    /// Reverse index: bucket chain links for each position
    links: Vec<SortLink>,
    /// List of non-empty buckets for efficient iteration
    used_buckets: Vec<usize>,
    /// Current search range [lo, hi)
    pub search_lo: i64,
    pub search_hi: i64,
}

impl SortInfo {
    /// Create a new empty sort index.
    pub fn new() -> Self {
        Self {
            vector: Vec::new(),
            abs_offset: 0,
            heads: vec![SortLink::NONE; NUM_BUCKETS],
            links: Vec::new(),
            used_buckets: Vec::new(),
            search_lo: 0,
            search_hi: 0,
        }
    }

    /// Create a sort index from a vector of samples.
    pub fn from_vector(vector: &[i16], abs_offset: i64) -> Self {
        let mut info = Self {
            vector: vector.to_vec(),
            abs_offset,
            heads: vec![SortLink::NONE; NUM_BUCKETS],
            links: Vec::with_capacity(vector.len()),
            used_buckets: Vec::new(),
            search_lo: abs_offset,
            search_hi: abs_offset + vector.len() as i64,
        };
        info.build_index();
        info
    }

    /// Build the hash table index.
    fn build_index(&mut self) {
        self.links.clear();
        self.links
            .resize(self.vector.len(), SortLink { index: 0, next: SortLink::NONE });
        self.used_buckets.clear();

        // Reset all heads
        for head in &mut self.heads {
            *head = SortLink::NONE;
        }

        // Build index by inserting each sample
        for (idx, &sample) in self.vector.iter().enumerate() {
            let bucket = sample_to_bucket(sample);

            // Track used buckets
            if self.heads[bucket] == SortLink::NONE {
                self.used_buckets.push(bucket);
            }

            // Insert at head of chain
            self.links[idx] = SortLink {
                index: idx,
                next: self.heads[bucket],
            };
            self.heads[bucket] = idx;
        }
    }

    /// Get the size of the indexed vector.
    #[inline]
    pub fn len(&self) -> usize {
        self.vector.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.vector.is_empty()
    }

    /// Set the search range (absolute positions).
    pub fn set_search_range(&mut self, lo: i64, hi: i64) {
        self.search_lo = lo.max(self.abs_offset);
        self.search_hi = hi.min(self.abs_offset + self.vector.len() as i64);
    }

    /// Find the first occurrence of a sample value within the search range.
    ///
    /// Returns the absolute position if found, or None.
    pub fn find_first(&self, value: i16) -> Option<i64> {
        let bucket = sample_to_bucket(value);
        let mut link_idx = self.heads[bucket];

        while link_idx != SortLink::NONE {
            let link = &self.links[link_idx];
            let abs_pos = self.abs_offset + link.index as i64;

            if abs_pos >= self.search_lo && abs_pos < self.search_hi {
                return Some(abs_pos);
            }

            link_idx = link.next;
        }

        None
    }

    /// Find all occurrences of a sample value within the search range.
    pub fn find_all(&self, value: i16) -> Vec<i64> {
        let bucket = sample_to_bucket(value);
        let mut results = Vec::new();
        let mut link_idx = self.heads[bucket];

        while link_idx != SortLink::NONE {
            let link = &self.links[link_idx];
            let abs_pos = self.abs_offset + link.index as i64;

            if abs_pos >= self.search_lo && abs_pos < self.search_hi {
                results.push(abs_pos);
            }

            link_idx = link.next;
        }

        results
    }

    /// Iterator over positions with a specific value within the search range.
    pub fn iter_matches(&self, value: i16) -> MatchIterator<'_> {
        let bucket = sample_to_bucket(value);
        MatchIterator {
            info: self,
            current: self.heads[bucket],
        }
    }

    /// Get a sample at an absolute position.
    pub fn get(&self, abs_pos: i64) -> Option<i16> {
        let idx = abs_pos - self.abs_offset;
        if idx >= 0 && (idx as usize) < self.vector.len() {
            Some(self.vector[idx as usize])
        } else {
            None
        }
    }

    /// Get a reference to the underlying vector.
    pub fn vector(&self) -> &[i16] {
        &self.vector
    }

    /// Clear and rebuild with new data.
    pub fn rebuild(&mut self, vector: &[i16], abs_offset: i64) {
        self.vector = vector.to_vec();
        self.abs_offset = abs_offset;
        self.search_lo = abs_offset;
        self.search_hi = abs_offset + vector.len() as i64;
        self.build_index();
    }

    /// Clear the index.
    pub fn clear(&mut self) {
        self.vector.clear();
        self.links.clear();
        self.used_buckets.clear();
        for head in &mut self.heads {
            *head = SortLink::NONE;
        }
        self.abs_offset = 0;
        self.search_lo = 0;
        self.search_hi = 0;
    }
}

impl Default for SortInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a 16-bit sample value to a bucket index (0-65535).
#[inline]
fn sample_to_bucket(sample: i16) -> usize {
    // Map -32768..32767 to 0..65535
    (sample as i32 + 32768) as usize
}

/// Iterator over positions matching a sample value.
pub struct MatchIterator<'a> {
    info: &'a SortInfo,
    current: usize,
}

impl Iterator for MatchIterator<'_> {
    type Item = i64;

    fn next(&mut self) -> Option<Self::Item> {
        while self.current != SortLink::NONE {
            let link = &self.info.links[self.current];
            let abs_pos = self.info.abs_offset + link.index as i64;
            self.current = link.next;

            if abs_pos >= self.info.search_lo && abs_pos < self.info.search_hi {
                return Some(abs_pos);
            }
        }
        None
    }
}

/// Find matching consecutive samples between two blocks.
///
/// Searches for a run of `min_match` consecutive matching samples starting
/// from `value` at position `pos_a` in block A, looking for matches in
/// the sort index of block B.
///
/// Returns the offset (pos_b - pos_a) if a match is found.
pub fn find_match(
    block_a: &[i16],
    pos_a: i64,
    block_a_offset: i64,
    sort_b: &SortInfo,
    min_match: usize,
) -> Option<i64> {
    // Get the starting sample from block A
    let idx_a = (pos_a - block_a_offset) as usize;
    if idx_a >= block_a.len() {
        return None;
    }
    let value = block_a[idx_a];

    // Search for this value in block B
    for pos_b in sort_b.iter_matches(value) {
        // Verify consecutive match
        let mut match_count = 1;
        let mut valid = true;

        for i in 1..min_match {
            let a_idx = idx_a + i;
            let b_pos = pos_b + i as i64;

            if a_idx >= block_a.len() {
                valid = false;
                break;
            }

            match sort_b.get(b_pos) {
                Some(b_val) if b_val == block_a[a_idx] => {
                    match_count += 1;
                }
                _ => {
                    valid = false;
                    break;
                }
            }
        }

        if valid && match_count >= min_match {
            return Some(pos_b - pos_a);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sort_info_creation() {
        let data = vec![100i16, 200, 100, 300, 100];
        let sort = SortInfo::from_vector(&data, 0);
        assert_eq!(sort.len(), 5);
    }

    #[test]
    fn test_find_first() {
        let data = vec![100i16, 200, 100, 300, 100];
        let sort = SortInfo::from_vector(&data, 10);

        // Value 100 appears at positions 10, 12, 14
        let first = sort.find_first(100);
        assert!(first.is_some());
        let pos = first.unwrap();
        assert!(pos == 10 || pos == 12 || pos == 14);

        // Value 200 appears at position 11
        assert_eq!(sort.find_first(200), Some(11));

        // Value 999 doesn't exist
        assert_eq!(sort.find_first(999), None);
    }

    #[test]
    fn test_find_all() {
        let data = vec![100i16, 200, 100, 300, 100];
        let sort = SortInfo::from_vector(&data, 10);

        let all = sort.find_all(100);
        assert_eq!(all.len(), 3);
        assert!(all.contains(&10));
        assert!(all.contains(&12));
        assert!(all.contains(&14));
    }

    #[test]
    fn test_search_range() {
        let data = vec![100i16, 200, 100, 300, 100];
        let mut sort = SortInfo::from_vector(&data, 10);

        // Full range
        assert_eq!(sort.find_all(100).len(), 3);

        // Restricted range
        sort.set_search_range(11, 14);
        assert_eq!(sort.find_all(100).len(), 1);
        assert_eq!(sort.find_first(100), Some(12));
    }

    #[test]
    fn test_sample_to_bucket() {
        assert_eq!(sample_to_bucket(-32768), 0);
        assert_eq!(sample_to_bucket(0), 32768);
        assert_eq!(sample_to_bucket(32767), 65535);
    }

    #[test]
    fn test_find_match() {
        let block_a = vec![1i16, 2, 3, 4, 5, 6, 7, 8];
        let block_b = vec![10i16, 20, 1, 2, 3, 4, 5, 30];
        let sort_b = SortInfo::from_vector(&block_b, 100);

        // Looking for match starting at pos 0 in A (value 1)
        // Should find it at pos 102 in B
        let offset = find_match(&block_a, 0, 0, &sort_b, 5);
        assert_eq!(offset, Some(102)); // 102 - 0 = 102
    }
}
