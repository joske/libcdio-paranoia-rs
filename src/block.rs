//! Data block structures for caching and verification.
//!
//! This module contains the core data structures used to cache raw CD reads
//! and track verified audio fragments.

#![allow(dead_code)]

use crate::{constants::CD_FRAMEWORDS, types::SampleFlags};

fn block_end(begin: i64, len: usize) -> i64 {
    begin.saturating_add(i64::try_from(len).unwrap_or(i64::MAX))
}

fn offset_index(begin: i64, pos: i64) -> Option<usize> {
    let offset = pos.checked_sub(begin)?;
    usize::try_from(offset).ok()
}

/// Raw CD read cache block.
///
/// Stores audio samples read from the CD along with per-sample metadata
/// for verification status.
#[derive(Debug)]
pub struct CBlock {
    /// Raw 16-bit audio samples
    pub vector: Vec<i16>,
    /// Absolute position in samples (from disc start)
    pub begin: i64,
    /// Per-sample flags (edge, blanked, verified)
    pub flags: Vec<SampleFlags>,
    /// Last sector number covered by this block
    pub lastsector: i64,
}

impl CBlock {
    /// Create a new empty cache block.
    #[must_use]
    pub fn new() -> Self {
        Self {
            vector: Vec::new(),
            begin: 0,
            flags: Vec::new(),
            lastsector: 0,
        }
    }

    /// Create a cache block with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(samples: usize) -> Self {
        Self {
            vector: Vec::with_capacity(samples),
            begin: 0,
            flags: Vec::with_capacity(samples),
            lastsector: 0,
        }
    }

    /// Create a cache block from raw sector data.
    #[must_use]
    pub fn from_sectors(data: &[i16], begin: i64, lastsector: i64) -> Self {
        let len = data.len();
        Self {
            vector: data.to_vec(),
            begin,
            flags: vec![SampleFlags::NONE; len],
            lastsector,
        }
    }

    /// Get the number of samples in this block.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.vector.len()
    }

    /// Check if the block is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vector.is_empty()
    }

    /// Get the ending position (exclusive) of this block.
    #[inline]
    #[must_use]
    pub fn end(&self) -> i64 {
        block_end(self.begin, self.vector.len())
    }

    /// Check if a position falls within this block.
    #[inline]
    #[must_use]
    pub fn contains(&self, pos: i64) -> bool {
        pos >= self.begin && pos < self.end()
    }

    /// Get a sample at an absolute position.
    #[must_use]
    pub fn get(&self, pos: i64) -> Option<i16> {
        if self.contains(pos) {
            offset_index(self.begin, pos)
                .and_then(|idx| self.vector.get(idx))
                .copied()
        } else {
            None
        }
    }

    /// Get flags at an absolute position.
    #[must_use]
    pub fn get_flags(&self, pos: i64) -> Option<SampleFlags> {
        if self.contains(pos) {
            offset_index(self.begin, pos)
                .and_then(|idx| self.flags.get(idx))
                .copied()
        } else {
            None
        }
    }

    /// Set flags at an absolute position.
    pub fn set_flags(&mut self, pos: i64, flags: SampleFlags) {
        if self.contains(pos) {
            if let Some(idx) = offset_index(self.begin, pos) {
                self.flags[idx] = flags;
            }
        }
    }

    /// Mark a range as verified.
    pub fn mark_verified(&mut self, start: i64, end: i64) {
        let start_offset = start.saturating_sub(self.begin);
        let end_offset = end.saturating_sub(self.begin);
        let mut start_idx = usize::try_from(start_offset).unwrap_or(0);
        start_idx = start_idx.min(self.flags.len());
        let mut end_idx = usize::try_from(end_offset).unwrap_or(self.flags.len());
        end_idx = end_idx.min(self.flags.len());
        for flag in &mut self.flags[start_idx..end_idx] {
            flag.0 |= SampleFlags::VERIFIED.0;
        }
    }

    /// Clear the block.
    pub fn clear(&mut self) {
        self.vector.clear();
        self.flags.clear();
        self.begin = 0;
        self.lastsector = 0;
    }
}

impl Default for CBlock {
    fn default() -> Self {
        Self::new()
    }
}

/// Verified data fragment.
///
/// Represents a contiguous segment of verified audio data that can be
/// merged into the final output. Each sample has associated flags
/// indicating verification status.
#[derive(Debug, Clone)]
pub struct VFragment {
    /// Verified 16-bit audio samples
    pub vector: Vec<i16>,
    /// Absolute position in samples
    pub begin: i64,
    /// Last sector covered by this fragment
    pub lastsector: i64,
    /// Per-sample verification flags
    pub flags: Vec<SampleFlags>,
}

impl VFragment {
    /// Create a new empty fragment.
    #[must_use]
    pub fn new() -> Self {
        Self {
            vector: Vec::new(),
            begin: 0,
            lastsector: 0,
            flags: Vec::new(),
        }
    }

    /// Create a fragment from a slice of samples (unverified).
    #[must_use]
    pub fn from_samples(data: &[i16], begin: i64, lastsector: i64) -> Self {
        let len = data.len();
        Self {
            vector: data.to_vec(),
            begin,
            lastsector,
            flags: vec![SampleFlags::NONE; len],
        }
    }

    /// Create a verified fragment from a slice of samples.
    #[must_use]
    pub fn from_samples_verified(data: &[i16], begin: i64, lastsector: i64) -> Self {
        let len = data.len();
        Self {
            vector: data.to_vec(),
            begin,
            lastsector,
            flags: vec![SampleFlags::VERIFIED; len],
        }
    }

    /// Create a fragment from a portion of a `CBlock`.
    #[must_use]
    pub fn from_cblock(block: &CBlock, start: i64, end: i64) -> Self {
        let start_offset = start.saturating_sub(block.begin);
        let end_offset = end.saturating_sub(block.begin);
        let mut start_idx = usize::try_from(start_offset).unwrap_or(0);
        start_idx = start_idx.min(block.vector.len());
        let mut end_idx = usize::try_from(end_offset).unwrap_or(block.vector.len());
        end_idx = end_idx.min(block.vector.len());
        Self {
            vector: block.vector[start_idx..end_idx].to_vec(),
            begin: start.max(block.begin),
            lastsector: block.lastsector,
            flags: block.flags[start_idx..end_idx].to_vec(),
        }
    }

    /// Get the number of samples.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.vector.len()
    }

    /// Check if empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vector.is_empty()
    }

    /// Get the ending position (exclusive).
    #[inline]
    #[must_use]
    pub fn end(&self) -> i64 {
        block_end(self.begin, self.vector.len())
    }

    /// Check if a position falls within this fragment.
    #[inline]
    #[must_use]
    pub fn contains(&self, pos: i64) -> bool {
        pos >= self.begin && pos < self.end()
    }

    /// Get a sample at an absolute position.
    #[must_use]
    pub fn get(&self, pos: i64) -> Option<i16> {
        if self.contains(pos) {
            offset_index(self.begin, pos)
                .and_then(|idx| self.vector.get(idx))
                .copied()
        } else {
            None
        }
    }

    /// Get flags at an absolute position.
    #[must_use]
    pub fn get_flags(&self, pos: i64) -> Option<SampleFlags> {
        if self.contains(pos) {
            offset_index(self.begin, pos)
                .and_then(|idx| self.flags.get(idx))
                .copied()
        } else {
            None
        }
    }

    /// Check if a sample is verified.
    #[must_use]
    pub fn is_verified(&self, pos: i64) -> bool {
        self.get_flags(pos)
            .is_some_and(super::types::SampleFlags::is_verified)
    }

    /// Check if all samples in the fragment are verified.
    #[must_use]
    pub fn all_verified(&self) -> bool {
        self.flags.iter().all(|f| f.is_verified())
    }

    /// Count verified samples.
    #[must_use]
    pub fn verified_count(&self) -> usize {
        self.flags.iter().filter(|f| f.is_verified()).count()
    }

    /// Mark a range as verified.
    pub fn mark_verified(&mut self, start: i64, end: i64) {
        let start_offset = start.saturating_sub(self.begin);
        let end_offset = end.saturating_sub(self.begin);
        let start_idx = usize::try_from(start_offset)
            .unwrap_or(0)
            .min(self.flags.len());
        let end_idx = usize::try_from(end_offset)
            .unwrap_or(self.flags.len())
            .min(self.flags.len());
        for flag in &mut self.flags[start_idx..end_idx] {
            flag.0 |= SampleFlags::VERIFIED.0;
        }
    }

    /// Mark edges (first and last N samples) as needing verification.
    pub fn mark_edges(&mut self, edge_size: usize) {
        let len = self.flags.len();
        // Mark leading edge
        for flag in self.flags.iter_mut().take(edge_size.min(len)) {
            flag.0 |= SampleFlags::EDGE.0;
            flag.0 &= !SampleFlags::VERIFIED.0; // Edges are not verified
        }
        // Mark trailing edge
        if len > edge_size {
            for flag in self.flags.iter_mut().skip(len - edge_size) {
                flag.0 |= SampleFlags::EDGE.0;
                flag.0 &= !SampleFlags::VERIFIED.0;
            }
        }
    }

    /// Try to merge another fragment into this one.
    /// Returns true if merge was successful (fragments overlap and match).
    pub fn try_merge(&mut self, other: &VFragment) -> bool {
        // Check for overlap
        let overlap_start = self.begin.max(other.begin);
        let overlap_end = self.end().min(other.end());

        if overlap_start >= overlap_end {
            // No overlap - check if contiguous
            if self.end() == other.begin {
                // Other follows self - append
                self.vector.extend_from_slice(&other.vector);
                self.flags.extend_from_slice(&other.flags);
                self.lastsector = self.lastsector.max(other.lastsector);
                return true;
            } else if other.end() == self.begin {
                // Self follows other - prepend
                let mut new_vec = other.vector.clone();
                new_vec.extend_from_slice(&self.vector);
                let mut new_flags = other.flags.clone();
                new_flags.extend_from_slice(&self.flags);
                self.vector = new_vec;
                self.flags = new_flags;
                self.begin = other.begin;
                return true;
            }
            return false;
        }

        // Verify overlapping region matches
        let self_start = (overlap_start - self.begin) as usize;
        let other_start = (overlap_start - other.begin) as usize;
        let overlap_len = (overlap_end - overlap_start) as usize;

        let self_slice = &self.vector[self_start..self_start + overlap_len];
        let other_slice = &other.vector[other_start..other_start + overlap_len];

        if self_slice != other_slice {
            return false; // Overlap doesn't match
        }

        // Mark overlapping region as verified in both
        for i in 0..overlap_len {
            self.flags[self_start + i].0 |= SampleFlags::VERIFIED.0;
        }

        // Extend if other has data beyond self
        if other.begin < self.begin {
            // Prepend data from other
            let prepend_len = (self.begin - other.begin) as usize;
            let mut new_vec = other.vector[..prepend_len].to_vec();
            new_vec.extend_from_slice(&self.vector);
            let mut new_flags = other.flags[..prepend_len].to_vec();
            new_flags.extend_from_slice(&self.flags);
            self.vector = new_vec;
            self.flags = new_flags;
            self.begin = other.begin;
        }

        if other.end() > self.end() {
            // Append data from other
            let append_start = (self.end() - other.begin) as usize;
            self.vector.extend_from_slice(&other.vector[append_start..]);
            self.flags.extend_from_slice(&other.flags[append_start..]);
        }

        self.lastsector = self.lastsector.max(other.lastsector);
        true
    }
}

impl Default for VFragment {
    fn default() -> Self {
        Self::new()
    }
}

/// Root block containing the verified output data.
///
/// This is the final destination for verified audio samples that will
/// be returned to the caller. Tracks per-sample verification status.
#[derive(Debug)]
pub struct RootBlock {
    /// Verified audio samples ready for output
    pub vector: Vec<i16>,
    /// Starting position in samples
    pub begin: i64,
    /// Position of last returned data
    pub returnedlimit: i64,
    /// Last sector covered
    pub lastsector: i64,
    /// Per-sample verification flags
    pub flags: Vec<SampleFlags>,
    /// Silence flags for each sample (for silence handling)
    pub silenceflag: Vec<bool>,
}

impl RootBlock {
    /// Create a new root block.
    #[must_use]
    pub fn new() -> Self {
        Self {
            vector: Vec::new(),
            begin: 0,
            returnedlimit: 0,
            lastsector: 0,
            flags: Vec::new(),
            silenceflag: Vec::new(),
        }
    }

    /// Create a root block with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(samples: usize) -> Self {
        Self {
            vector: Vec::with_capacity(samples),
            begin: 0,
            returnedlimit: 0,
            lastsector: 0,
            flags: Vec::with_capacity(samples),
            silenceflag: Vec::with_capacity(samples),
        }
    }

    /// Get the number of samples.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.vector.len()
    }

    /// Check if empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vector.is_empty()
    }

    /// Get the ending position (exclusive).
    #[inline]
    #[must_use]
    pub fn end(&self) -> i64 {
        block_end(self.begin, self.vector.len())
    }

    /// Clear the root block.
    pub fn clear(&mut self) {
        self.vector.clear();
        self.flags.clear();
        self.silenceflag.clear();
        self.begin = 0;
        self.returnedlimit = 0;
        self.lastsector = 0;
    }

    /// Extract a frame of audio data starting at the given position.
    ///
    /// Returns `CD_FRAMEWORDS` samples if available.
    #[must_use]
    pub fn extract_frame(&self, pos: i64) -> Option<&[i16]> {
        let start = offset_index(self.begin, pos)?;
        let end = start.checked_add(CD_FRAMEWORDS)?;
        if end <= self.vector.len() {
            Some(&self.vector[start..end])
        } else {
            None
        }
    }

    /// Check if a frame is fully verified.
    #[must_use]
    pub fn is_frame_verified(&self, pos: i64) -> bool {
        let Some(start) = offset_index(self.begin, pos) else {
            return false;
        };
        let end = start + CD_FRAMEWORDS;
        if end > self.flags.len() {
            return false;
        }
        self.flags[start..end].iter().all(|f| f.is_verified())
    }

    /// Count verified samples in a frame.
    #[must_use]
    pub fn frame_verified_count(&self, pos: i64) -> usize {
        let Some(start) = offset_index(self.begin, pos) else {
            return 0;
        };
        let end = (start + CD_FRAMEWORDS).min(self.flags.len());
        if start >= self.flags.len() {
            return 0;
        }
        self.flags[start..end]
            .iter()
            .filter(|f| f.is_verified())
            .count()
    }

    /// Merge a verified fragment into the root block.
    /// Only copies data where the fragment has verified samples or
    /// where root has unverified samples.
    pub fn merge_fragment(&mut self, fragment: &VFragment) {
        if fragment.is_empty() {
            return;
        }

        if self.is_empty() {
            // Initialize from fragment
            self.vector.clone_from(&fragment.vector);
            self.flags.clone_from(&fragment.flags);
            self.begin = fragment.begin;
            self.lastsector = fragment.lastsector;
            self.silenceflag = vec![false; fragment.vector.len()];
            return;
        }

        let overlap_start = self.begin.max(fragment.begin);
        let overlap_end = self.end().min(fragment.end());

        // Handle prepending
        if fragment.begin < self.begin {
            let prepend_len = (self.begin - fragment.begin) as usize;
            let mut new_vec = fragment.vector[..prepend_len].to_vec();
            new_vec.extend_from_slice(&self.vector);
            let mut new_flags = fragment.flags[..prepend_len].to_vec();
            new_flags.extend_from_slice(&self.flags);
            let mut new_silence = vec![false; prepend_len];
            new_silence.extend_from_slice(&self.silenceflag);
            self.vector = new_vec;
            self.flags = new_flags;
            self.silenceflag = new_silence;
            self.begin = fragment.begin;
        }

        // Handle appending
        if fragment.end() > self.end() {
            let append_start = (self.end() - fragment.begin) as usize;
            self.vector
                .extend_from_slice(&fragment.vector[append_start..]);
            self.flags
                .extend_from_slice(&fragment.flags[append_start..]);
            self.silenceflag.extend(std::iter::repeat_n(
                false,
                fragment.vector.len() - append_start,
            ));
        }

        // Merge overlapping region - prefer verified data
        if overlap_start < overlap_end {
            let root_start = (overlap_start - self.begin) as usize;
            let frag_start = (overlap_start - fragment.begin) as usize;
            let overlap_len = (overlap_end - overlap_start) as usize;

            for i in 0..overlap_len {
                let root_idx = root_start + i;
                let frag_idx = frag_start + i;

                // If fragment sample is verified and root isn't, use fragment
                // If both verified, mark as verified (they should match)
                // If fragment verified, use fragment data and mark verified
                if fragment.flags[frag_idx].is_verified() {
                    if !self.flags[root_idx].is_verified() {
                        // Replace with verified data
                        self.vector[root_idx] = fragment.vector[frag_idx];
                    }
                    // Mark as verified
                    self.flags[root_idx].0 |= SampleFlags::VERIFIED.0;
                }
            }
        }

        self.lastsector = self.lastsector.max(fragment.lastsector);
    }
}

impl Default for RootBlock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cblock_creation() {
        let data = vec![1i16, 2, 3, 4, 5];
        let block = CBlock::from_sectors(&data, 100, 0);
        assert_eq!(block.len(), 5);
        assert_eq!(block.begin, 100);
        assert_eq!(block.end(), 105);
    }

    #[test]
    fn test_cblock_contains() {
        let data = vec![1i16, 2, 3, 4, 5];
        let block = CBlock::from_sectors(&data, 100, 0);
        assert!(block.contains(100));
        assert!(block.contains(104));
        assert!(!block.contains(99));
        assert!(!block.contains(105));
    }

    #[test]
    fn test_cblock_get() {
        let data = vec![10i16, 20, 30, 40, 50];
        let block = CBlock::from_sectors(&data, 100, 0);
        assert_eq!(block.get(100), Some(10));
        assert_eq!(block.get(102), Some(30));
        assert_eq!(block.get(99), None);
    }

    #[test]
    fn test_vfragment_from_cblock() {
        let data = vec![1i16, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let block = CBlock::from_sectors(&data, 100, 0);
        let frag = VFragment::from_cblock(&block, 102, 107);
        assert_eq!(frag.len(), 5);
        assert_eq!(frag.begin, 102);
        assert_eq!(frag.vector, vec![3, 4, 5, 6, 7]);
    }
}
