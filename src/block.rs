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
/// merged into the final output.
#[derive(Debug)]
pub struct VFragment {
    /// Verified 16-bit audio samples
    pub vector: Vec<i16>,
    /// Absolute position in samples
    pub begin: i64,
    /// Last sector covered by this fragment
    pub lastsector: i64,
}

impl VFragment {
    /// Create a new empty fragment.
    #[must_use]
    pub fn new() -> Self {
        Self {
            vector: Vec::new(),
            begin: 0,
            lastsector: 0,
        }
    }

    /// Create a fragment from a slice of samples.
    #[must_use]
    pub fn from_samples(data: &[i16], begin: i64, lastsector: i64) -> Self {
        Self {
            vector: data.to_vec(),
            begin,
            lastsector,
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
}

impl Default for VFragment {
    fn default() -> Self {
        Self::new()
    }
}

/// Root block containing the verified output data.
///
/// This is the final destination for verified audio samples that will
/// be returned to the caller.
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
    /// Silence flags for each sample
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
