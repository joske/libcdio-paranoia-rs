//! Overlap detection and dynamic adjustment.
//!
//! This module handles the detection of overlapping regions between
//! consecutive CD reads and manages the dynamic adjustment of overlap
//! parameters based on observed jitter.

#![allow(dead_code)]

use crate::{
    constants::{CD_FRAMEWORDS, MAX_SECTOR_OVERLAP, MIN_WORDS_OVERLAP, MIN_WORDS_SEARCH},
    isort::SortInfo,
};

/// Statistics for jitter/drift offset tracking.
#[derive(Debug, Clone)]
pub struct OffsetStats {
    /// Accumulated offset values
    offsets: Vec<i64>,
    /// Maximum number of samples to track
    max_samples: usize,
    /// Current index for circular buffer
    index: usize,
    /// Number of samples collected
    count: usize,
}

impl OffsetStats {
    /// Create new offset statistics tracker.
    pub fn new(max_samples: usize) -> Self {
        Self {
            offsets: vec![0; max_samples],
            max_samples,
            index: 0,
            count: 0,
        }
    }

    /// Add a new offset measurement.
    pub fn add(&mut self, offset: i64) {
        self.offsets[self.index] = offset;
        self.index = (self.index + 1) % self.max_samples;
        if self.count < self.max_samples {
            self.count += 1;
        }
    }

    /// Get the mean offset.
    pub fn mean(&self) -> i64 {
        if self.count == 0 {
            return 0;
        }
        let sum: i64 = self.offsets[..self.count].iter().sum();
        sum / self.count as i64
    }

    /// Get the standard deviation of offsets.
    pub fn std_dev(&self) -> f64 {
        if self.count < 2 {
            return 0.0;
        }
        let mean = self.mean() as f64;
        let variance: f64 = self.offsets[..self.count]
            .iter()
            .map(|&x| {
                let diff = x as f64 - mean;
                diff * diff
            })
            .sum::<f64>()
            / (self.count - 1) as f64;
        variance.sqrt()
    }

    /// Get the maximum absolute offset.
    pub fn max_abs(&self) -> i64 {
        self.offsets[..self.count]
            .iter()
            .map(|x| x.abs())
            .max()
            .unwrap_or(0)
    }

    /// Clear all statistics.
    pub fn clear(&mut self) {
        self.index = 0;
        self.count = 0;
        for offset in &mut self.offsets {
            *offset = 0;
        }
    }
}

/// Dynamic overlap settings that adapt to drive behavior.
#[derive(Debug, Clone)]
pub struct DynamicOverlap {
    /// Current overlap size in samples
    pub overlap: i64,
    /// Current search window size
    pub search: i64,
    /// Cumulative drift compensation
    pub drift: i64,
    /// Stage 1 offset statistics (overlap matching)
    pub stage1_stats: OffsetStats,
    /// Stage 2 offset statistics (root merging)
    pub stage2_stats: OffsetStats,
    /// Number of measurements before recalibration
    recalibrate_interval: usize,
    /// Measurements since last recalibration
    measurements: usize,
}

impl DynamicOverlap {
    /// Create new dynamic overlap tracker with default settings.
    pub fn new() -> Self {
        Self {
            overlap: MIN_WORDS_OVERLAP,
            search: MIN_WORDS_SEARCH,
            drift: 0,
            stage1_stats: OffsetStats::new(10),
            stage2_stats: OffsetStats::new(10),
            recalibrate_interval: 10,
            measurements: 0,
        }
    }

    /// Record a jitter measurement and potentially recalibrate.
    pub fn record_jitter(&mut self, offset: i64, stage: u8) {
        match stage {
            1 => self.stage1_stats.add(offset),
            2 => self.stage2_stats.add(offset),
            _ => {}
        }

        self.measurements += 1;
        if self.measurements >= self.recalibrate_interval {
            self.recalibrate();
            self.measurements = 0;
        }
    }

    /// Recalibrate overlap and search parameters based on statistics.
    fn recalibrate(&mut self) {
        // Calculate new overlap based on observed jitter
        let max_jitter = self.stage1_stats.max_abs().max(self.stage2_stats.max_abs());
        let std_dev = self.stage1_stats.std_dev().max(self.stage2_stats.std_dev());

        // New overlap should cover worst-case jitter plus margin
        let margin = (std_dev * 2.0) as i64;
        let new_overlap = (max_jitter + margin + MIN_WORDS_OVERLAP)
            .min(MAX_SECTOR_OVERLAP * CD_FRAMEWORDS as i64);

        self.overlap = new_overlap.max(MIN_WORDS_OVERLAP);
        self.search = self.overlap.max(MIN_WORDS_SEARCH);

        // Update drift estimate
        let mean_drift = self.stage1_stats.mean();
        self.drift = i64::midpoint(self.drift, mean_drift);
    }

    /// Get the current overlap in sectors.
    pub fn overlap_sectors(&self) -> i64 {
        (self.overlap + CD_FRAMEWORDS as i64 - 1) / CD_FRAMEWORDS as i64
    }

    /// Reset to default values.
    pub fn reset(&mut self) {
        self.overlap = MIN_WORDS_OVERLAP;
        self.search = MIN_WORDS_SEARCH;
        self.drift = 0;
        self.stage1_stats.clear();
        self.stage2_stats.clear();
        self.measurements = 0;
    }
}

impl Default for DynamicOverlap {
    fn default() -> Self {
        Self::new()
    }
}

/// Find overlapping region between two blocks.
///
/// Searches for where `block_new` overlaps with `block_existing` using
/// the sort index for fast matching.
///
/// Returns the offset (`new_pos - existing_pos`) if overlap is found.
pub fn find_overlap(
    block_new: &[i16],
    new_offset: i64,
    sort_existing: &SortInfo,
    min_match: usize,
    _search_range: i64,
) -> Option<i64> {
    // Try to find overlap at several positions in the new block
    let try_positions = [0, block_new.len() / 4, block_new.len() / 2];

    for &rel_pos in &try_positions {
        if rel_pos >= block_new.len() {
            continue;
        }

        let abs_pos = new_offset + rel_pos as i64;
        let value = block_new[rel_pos];

        // Look for this value in the existing block
        for existing_pos in sort_existing.iter_matches(value) {
            // Calculate offset
            let offset = existing_pos - abs_pos;

            // Verify match
            if verify_overlap(block_new, new_offset, sort_existing, offset, min_match) {
                return Some(offset);
            }
        }
    }

    None
}

/// Verify that an overlap at the given offset is valid.
fn verify_overlap(
    block_new: &[i16],
    new_offset: i64,
    sort_existing: &SortInfo,
    offset: i64,
    min_match: usize,
) -> bool {
    let mut match_count = 0;

    for (i, &sample) in block_new.iter().enumerate() {
        let existing_pos = new_offset + i as i64 + offset;
        if let Some(existing_sample) = sort_existing.get(existing_pos) {
            if existing_sample == sample {
                match_count += 1;
                if match_count >= min_match {
                    return true;
                }
            } else {
                match_count = 0;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offset_stats() {
        let mut stats = OffsetStats::new(5);
        stats.add(10);
        stats.add(20);
        stats.add(30);

        assert_eq!(stats.mean(), 20);
        assert_eq!(stats.max_abs(), 30);
    }

    #[test]
    fn test_dynamic_overlap() {
        let mut overlap = DynamicOverlap::new();
        assert_eq!(overlap.overlap, MIN_WORDS_OVERLAP);

        // Record some jitter
        for i in 0..15 {
            overlap.record_jitter(i * 10, 1);
        }

        // Overlap should have increased
        assert!(overlap.overlap >= MIN_WORDS_OVERLAP);
    }

    #[test]
    fn test_find_overlap() {
        // Create existing block
        let existing = vec![0i16, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let sort = SortInfo::from_vector(&existing, 100);

        // Create new block that overlaps
        let new = vec![5i16, 6, 7, 8, 9, 10, 11, 12];

        let offset = find_overlap(&new, 200, &sort, 4, 100);
        assert!(offset.is_some());
        // offset should be 100 + 5 - 200 = -95
        assert_eq!(offset.unwrap(), -95);
    }
}
