//! Multi-read consensus voting for sample-level verification.
//!
//! When simple double-read verification fails, this module collects multiple
//! reads and uses majority voting to determine the correct value for each sample.

#![allow(dead_code)]

use std::collections::HashMap;

use crate::types::SampleFlags;

/// Minimum number of reads needed for consensus voting.
pub const MIN_READS_FOR_CONSENSUS: usize = 3;

/// Minimum vote ratio to consider a sample verified (e.g., 0.5 = majority).
pub const MIN_VOTE_RATIO: f64 = 0.5;

/// High confidence threshold - if this many reads agree, sample is definitely correct.
pub const HIGH_CONFIDENCE_RATIO: f64 = 0.75;

/// A single read result for consensus building.
#[derive(Debug, Clone)]
pub struct ReadResult {
    /// Sample data from this read
    pub samples: Vec<i16>,
    /// Absolute position of first sample
    pub begin: i64,
    /// Jitter offset detected for this read (0 if aligned)
    pub jitter: i64,
}

impl ReadResult {
    /// Create a new read result.
    pub fn new(samples: Vec<i16>, begin: i64, jitter: i64) -> Self {
        Self {
            samples,
            begin,
            jitter,
        }
    }

    /// Get sample at absolute position, accounting for jitter.
    pub fn get(&self, pos: i64) -> Option<i16> {
        let adjusted_pos = pos - self.jitter;
        let offset = adjusted_pos - self.begin;
        if offset >= 0 && (offset as usize) < self.samples.len() {
            Some(self.samples[offset as usize])
        } else {
            None
        }
    }

    /// Get the end position.
    pub fn end(&self) -> i64 {
        self.begin + self.samples.len() as i64
    }
}

/// Result of consensus voting for a single sample.
#[derive(Debug, Clone, Copy)]
pub struct SampleVote {
    /// The winning sample value
    pub value: i16,
    /// Number of reads that agreed on this value
    pub votes: usize,
    /// Total number of reads that covered this position
    pub total: usize,
    /// Confidence level (0.0 - 1.0)
    pub confidence: f64,
}

impl SampleVote {
    /// Check if this vote meets the minimum confidence threshold.
    #[inline]
    pub fn is_verified(&self) -> bool {
        self.confidence >= MIN_VOTE_RATIO
    }

    /// Check if this vote has high confidence.
    #[inline]
    pub fn is_high_confidence(&self) -> bool {
        self.confidence >= HIGH_CONFIDENCE_RATIO
    }
}

/// Consensus builder that collects multiple reads and performs voting.
#[derive(Debug)]
pub struct ConsensusBuilder {
    /// Collected read results
    reads: Vec<ReadResult>,
    /// Start position of the region we're building consensus for
    region_begin: i64,
    /// End position of the region
    region_end: i64,
}

impl ConsensusBuilder {
    /// Create a new consensus builder for a specific region.
    pub fn new(region_begin: i64, region_end: i64) -> Self {
        Self {
            reads: Vec::new(),
            region_begin,
            region_end,
        }
    }

    /// Add a read result to the consensus.
    pub fn add_read(&mut self, result: ReadResult) {
        self.reads.push(result);
    }

    /// Get the number of reads collected.
    pub fn read_count(&self) -> usize {
        self.reads.len()
    }

    /// Check if we have enough reads for consensus.
    pub fn has_enough_reads(&self) -> bool {
        self.reads.len() >= MIN_READS_FOR_CONSENSUS
    }

    /// Perform consensus voting for a single sample position.
    fn vote_sample(&self, pos: i64) -> Option<SampleVote> {
        let mut votes: HashMap<i16, usize> = HashMap::new();
        let mut total = 0;

        for read in &self.reads {
            if let Some(sample) = read.get(pos) {
                *votes.entry(sample).or_insert(0) += 1;
                total += 1;
            }
        }

        if total == 0 {
            return None;
        }

        // Find the value with the most votes
        let (value, vote_count) = votes.into_iter().max_by_key(|(_, count)| *count)?;

        let confidence = vote_count as f64 / total as f64;

        Some(SampleVote {
            value,
            votes: vote_count,
            total,
            confidence,
        })
    }

    /// Build consensus for the entire region.
    /// Returns (samples, flags) where flags indicate verification confidence.
    pub fn build_consensus(&self) -> Option<(Vec<i16>, Vec<SampleFlags>)> {
        if !self.has_enough_reads() {
            return None;
        }

        let len = (self.region_end - self.region_begin) as usize;
        let mut samples = Vec::with_capacity(len);
        let mut flags = Vec::with_capacity(len);

        for i in 0..len {
            let pos = self.region_begin + i as i64;
            if let Some(vote) = self.vote_sample(pos) {
                samples.push(vote.value);

                let flag = if vote.is_high_confidence() {
                    SampleFlags::VERIFIED
                } else if vote.is_verified() {
                    // Verified but not high confidence - mark as edge
                    SampleFlags(SampleFlags::VERIFIED.0 | SampleFlags::EDGE.0)
                } else {
                    // Not enough agreement - unverified
                    SampleFlags::NONE
                };
                flags.push(flag);
            } else {
                // No data for this position - use zero
                samples.push(0);
                flags.push(SampleFlags::BLANKED);
            }
        }

        Some((samples, flags))
    }

    /// Build consensus and return statistics.
    pub fn build_with_stats(&self) -> Option<ConsensusResult> {
        let (samples, flags) = self.build_consensus()?;

        let verified_count = flags.iter().filter(|f| f.is_verified()).count();
        let high_confidence_count = flags
            .iter()
            .filter(|f| f.0 == SampleFlags::VERIFIED.0)
            .count();
        let blanked_count = flags.iter().filter(|f| f.is_blanked()).count();

        Some(ConsensusResult {
            samples,
            flags,
            begin: self.region_begin,
            read_count: self.reads.len(),
            verified_count,
            high_confidence_count,
            blanked_count,
        })
    }

    /// Find positions where reads disagree.
    pub fn find_disagreements(&self) -> Vec<i64> {
        let mut disagreements = Vec::new();

        for i in 0..(self.region_end - self.region_begin) as usize {
            let pos = self.region_begin + i as i64;
            if let Some(vote) = self.vote_sample(pos) {
                if !vote.is_verified() {
                    disagreements.push(pos);
                }
            }
        }

        disagreements
    }
}

/// Result of consensus building with statistics.
#[derive(Debug)]
pub struct ConsensusResult {
    /// Consensus sample values
    pub samples: Vec<i16>,
    /// Per-sample flags
    pub flags: Vec<SampleFlags>,
    /// Start position
    pub begin: i64,
    /// Number of reads used
    pub read_count: usize,
    /// Number of verified samples
    pub verified_count: usize,
    /// Number of high-confidence samples
    pub high_confidence_count: usize,
    /// Number of blanked (missing) samples
    pub blanked_count: usize,
}

impl ConsensusResult {
    /// Get the end position.
    pub fn end(&self) -> i64 {
        self.begin + self.samples.len() as i64
    }

    /// Get verification percentage.
    pub fn verified_percent(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        (self.verified_count as f64 / self.samples.len() as f64) * 100.0
    }

    /// Check if consensus is acceptable (most samples verified).
    pub fn is_acceptable(&self) -> bool {
        self.verified_percent() >= 90.0
    }
}

/// Quick consensus check between two reads.
/// Returns the positions where they disagree.
pub fn find_mismatches(read1: &[i16], read2: &[i16]) -> Vec<usize> {
    read1
        .iter()
        .zip(read2.iter())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect()
}

/// Vote between three samples.
/// Returns the majority value, or `a` if no majority.
pub fn vote_three(a: i16, b: i16, c: i16) -> i16 {
    if a == b || a == c {
        a
    } else if b == c {
        b
    } else {
        a // No majority, return first value
    }
}

/// Perform sample-by-sample voting between multiple reads.
/// All reads must have the same length.
pub fn vote_samples(reads: &[&[i16]]) -> Option<Vec<i16>> {
    if reads.is_empty() {
        return None;
    }

    let len = reads[0].len();
    if reads.iter().any(|r| r.len() != len) {
        return None; // All reads must have same length
    }

    let mut result = Vec::with_capacity(len);

    for i in 0..len {
        let mut votes: HashMap<i16, usize> = HashMap::new();
        for read in reads {
            *votes.entry(read[i]).or_insert(0) += 1;
        }

        // Pick the value with most votes
        let winner = votes
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map_or(0, |(value, _)| value);

        result.push(winner);
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vote_three() {
        assert_eq!(vote_three(1, 1, 2), 1);
        assert_eq!(vote_three(1, 2, 1), 1);
        assert_eq!(vote_three(2, 1, 1), 1);
        assert_eq!(vote_three(1, 2, 3), 1); // No majority, returns a
    }

    #[test]
    fn test_vote_samples() {
        let read1: Vec<i16> = vec![1, 2, 3, 4, 5];
        let read2: Vec<i16> = vec![1, 2, 3, 4, 5];
        let read3: Vec<i16> = vec![1, 9, 3, 4, 5]; // Differs at position 1

        let result = vote_samples(&[&read1, &read2, &read3]).unwrap();
        assert_eq!(result, vec![1, 2, 3, 4, 5]); // Majority wins
    }

    #[test]
    fn test_consensus_builder() {
        let mut builder = ConsensusBuilder::new(0, 10);

        // Add three reads
        builder.add_read(ReadResult::new(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0, 0));
        builder.add_read(ReadResult::new(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0, 0));
        builder.add_read(ReadResult::new(vec![1, 2, 99, 4, 5, 6, 7, 8, 9, 10], 0, 0));

        assert!(builder.has_enough_reads());

        let result = builder.build_with_stats().unwrap();
        assert_eq!(result.samples[2], 3); // Majority wins over 99
        assert!(result.verified_percent() > 90.0);
    }

    #[test]
    fn test_read_result_with_jitter() {
        let read = ReadResult::new(vec![10, 20, 30, 40, 50], 100, 2);

        // With jitter of 2, position 102 should map to sample at index 0
        assert_eq!(read.get(102), Some(10));
        assert_eq!(read.get(104), Some(30));
    }

    #[test]
    fn test_find_mismatches() {
        let read1 = vec![1, 2, 3, 4, 5];
        let read2 = vec![1, 9, 3, 9, 5];

        let mismatches = find_mismatches(&read1, &read2);
        assert_eq!(mismatches, vec![1, 3]);
    }

    #[test]
    fn test_sample_vote_confidence() {
        let vote = SampleVote {
            value: 42,
            votes: 3,
            total: 4,
            confidence: 0.75,
        };

        assert!(vote.is_verified());
        assert!(vote.is_high_confidence());

        let low_vote = SampleVote {
            value: 42,
            votes: 2,
            total: 5,
            confidence: 0.4,
        };

        assert!(!low_vote.is_verified());
        assert!(!low_vote.is_high_confidence());
    }
}
