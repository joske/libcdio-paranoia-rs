//! Core paranoia state machine for verified CD audio reading.
//!
//! This module implements the main paranoia algorithm with two-stage
//! verification, automatic jitter correction, and rift repair.

use std::io::SeekFrom;

use crate::{
    backoff::{BackoffAction, BackoffStrategy},
    block::{CBlock, RootBlock, VFragment},
    cdda::CdromDrive,
    consensus::{ConsensusBuilder, ReadResult, MIN_READS_FOR_CONSENSUS},
    constants::{CACHEMODEL_SECTORS, CD_FRAMEWORDS, MIN_WORDS_OVERLAP, MIN_WORDS_SEARCH},
    error::{Error, Result},
    gap::{self, RiftType},
    isort::{self, SortInfo},
    overlap::{self, DynamicOverlap},
    types::{Lsn, ParanoiaCallback, ParanoiaMode},
};

/// Threshold for considering samples as silence (absolute value).
/// Samples with |value| <= `SILENCE_THRESHOLD` are considered silent.
const SILENCE_THRESHOLD: i16 = 5;

/// Detect jitter offset between reference and test reads (standalone function).
fn detect_jitter_offset_static(reference: &[i16], test: &[i16], search_range: i64) -> i64 {
    // Quick check - are they identical?
    if reference == test {
        return 0;
    }

    let match_len = MIN_WORDS_OVERLAP as usize;
    if reference.len() < match_len || test.len() < match_len {
        return 0;
    }

    // Sample from middle of block for matching
    let check_pos = reference.len() / 2;
    if check_pos + match_len > reference.len() {
        return 0;
    }

    let ref_slice = &reference[check_pos..check_pos + match_len];

    // Search for this pattern in test with offset
    for offset in 0..=search_range {
        for sign in &[1i64, -1i64] {
            let actual_offset = offset * sign;
            let test_pos = (check_pos as i64 + actual_offset) as usize;

            if test_pos + match_len <= test.len() {
                let test_slice = &test[test_pos..test_pos + match_len];
                if ref_slice == test_slice {
                    return actual_offset;
                }
            }
        }
    }

    0 // No jitter detected
}

/// A cached block with its sort index for fast matching.
struct CachedBlock {
    /// The raw audio data
    block: CBlock,
    /// Sort index for O(1) sample lookup
    sort: SortInfo,
}

/// Callback function type for progress reporting.
pub type CallbackFn = Box<dyn FnMut(i64, ParanoiaCallback)>;

/// Main paranoia state machine.
///
/// Manages the verified reading of CD audio data with automatic
/// error detection and correction.
///
/// Note: Paranoia does NOT own the drive - the caller is responsible
/// for keeping the drive alive while paranoia is in use, and for
/// freeing the drive separately after paranoia is freed.
#[allow(dead_code)]
pub struct Paranoia {
    /// Pointer to the CD drive being read (not owned)
    drive: *mut CdromDrive,
    /// Current paranoia mode flags
    mode: ParanoiaMode,
    /// Current read position (in samples)
    cursor: i64,
    /// First sector of the reading range
    first_sector: Lsn,
    /// Last sector of the reading range
    last_sector: Lsn,
    /// Cache of raw read blocks with sort indices
    cache: Vec<CachedBlock>,
    /// Verified fragments
    fragments: Vec<VFragment>,
    /// Root block with verified output
    root: RootBlock,
    /// Sort index for current cache
    sort_cache: Option<SortInfo>,
    /// Dynamic overlap settings
    dynoverlap: DynamicOverlap,
    /// Jitter measurement
    jitter: i64,
    /// Cache model size in sectors
    cache_model_sectors: i32,
    /// Maximum retries for failed reads
    max_retries: i32,
    /// Enable skipping on failure
    enable_skip: bool,
    /// Callback for progress reporting
    callback: Option<CallbackFn>,
    /// Output buffer for the current frame
    output_buffer: Vec<i16>,
    /// Backoff strategy for handling persistent errors
    backoff: BackoffStrategy,
}

// Safety: The raw pointer is only used for reading from the drive,
// and the caller guarantees the drive outlives the paranoia instance.
unsafe impl Send for Paranoia {}

impl Paranoia {
    /// Create a new paranoia instance from a raw drive pointer.
    ///
    /// # Safety
    /// The caller must ensure:
    /// - `drive` is a valid pointer to a `CdromDrive`
    /// - The drive outlives this paranoia instance
    /// - The drive is not freed while paranoia is in use
    pub unsafe fn from_drive_ptr(drive: *mut CdromDrive) -> Self {
        let drive_ref = unsafe { &*drive };
        let first_sector = drive_ref.disc_first_sector();
        let last_sector = drive_ref.disc_last_sector();

        Self {
            drive,
            mode: ParanoiaMode::FULL,
            cursor: first_sector as i64 * CD_FRAMEWORDS as i64,
            first_sector,
            last_sector,
            cache: Vec::new(),
            fragments: Vec::new(),
            root: RootBlock::new(),
            sort_cache: None,
            dynoverlap: DynamicOverlap::new(),
            jitter: 0,
            cache_model_sectors: CACHEMODEL_SECTORS,
            max_retries: 20,
            enable_skip: true,
            callback: None,
            output_buffer: vec![0i16; CD_FRAMEWORDS],
            backoff: BackoffStrategy::new(),
        }
    }

    /// Create a new paranoia instance for the given drive.
    ///
    /// Note: This leaks the drive - use `from_drive_ptr` for FFI.
    #[must_use]
    pub fn new(drive: CdromDrive) -> Self {
        // Leak the drive to get a stable pointer
        let drive_ptr = Box::into_raw(Box::new(drive));
        // Safety: we just created this pointer
        unsafe { Self::from_drive_ptr(drive_ptr) }
    }

    /// Set the paranoia mode.
    pub fn set_mode(&mut self, mode: ParanoiaMode) {
        self.mode = mode;
        self.enable_skip = !mode.contains(ParanoiaMode::NEVERSKIP);
    }

    /// Get the current paranoia mode.
    #[must_use]
    pub fn mode(&self) -> ParanoiaMode {
        self.mode
    }

    /// Seek to a position.
    ///
    /// `seek` is in sectors, `whence` follows the standard seek semantics:
    /// - `SEEK_SET`: absolute position
    /// - `SEEK_CUR`: relative to current
    /// - `SEEK_END`: relative to end
    #[must_use]
    pub fn seek(&mut self, seek: i32, whence: SeekFrom) -> Lsn {
        let target_sector = match whence {
            SeekFrom::Start(_) => self.first_sector + seek,
            SeekFrom::Current(_) => (self.cursor / CD_FRAMEWORDS as i64) as Lsn + seek,
            SeekFrom::End(_) => self.last_sector + seek,
        };

        let target_sector = target_sector.clamp(self.first_sector, self.last_sector);
        self.cursor = target_sector as i64 * CD_FRAMEWORDS as i64;

        // Clear cache when seeking
        self.cache.clear();
        self.fragments.clear();
        self.root.clear();
        self.sort_cache = None;

        target_sector
    }

    /// Set the reading range.
    pub fn set_range(&mut self, start: Lsn, end: Lsn) {
        let drive = unsafe { &*self.drive };
        self.first_sector = start.max(drive.disc_first_sector());
        self.last_sector = end.min(drive.disc_last_sector());

        // Reset cursor if out of range
        let cursor_sector = (self.cursor / CD_FRAMEWORDS as i64) as Lsn;
        if cursor_sector < self.first_sector || cursor_sector > self.last_sector {
            self.cursor = self.first_sector as i64 * CD_FRAMEWORDS as i64;
        }
    }

    /// Set a callback for progress reporting.
    pub fn set_callback<F>(&mut self, callback: F)
    where
        F: FnMut(i64, ParanoiaCallback) + 'static,
    {
        self.callback = Some(Box::new(callback));
    }

    /// Clear the callback.
    pub fn clear_callback(&mut self) {
        self.callback = None;
    }

    /// Set the overlap amount (for debugging/testing).
    pub fn set_overlap(&mut self, overlap: i64) {
        self.dynoverlap.overlap = overlap.max(MIN_WORDS_OVERLAP);
    }

    /// Set or query the cache model size.
    ///
    /// Pass -1 to query without changing. Returns the previous value.
    pub fn cache_model_size(&mut self, sectors: i32) -> i32 {
        let old = self.cache_model_sectors;
        if sectors >= 0 {
            self.cache_model_sectors = sectors;
        }
        old
    }

    /// Read the next sector of verified audio data.
    ///
    /// Returns a slice of `CD_FRAMEWORDS` samples (1176 16-bit values).
    ///
    /// # Errors
    ///
    /// Propagates any error returned by `read_limited`.
    pub fn read(&mut self) -> Result<&[i16]> {
        let max_retries = self.max_retries;
        self.read_limited(max_retries)
    }

    /// Read the next sector with a specific retry limit.
    ///
    /// # Errors
    ///
    /// Returns an error if the read position is outside the configured range
    /// or if the drive fails and skipping is disabled.
    pub fn read_limited(&mut self, max_retries: i32) -> Result<&[i16]> {
        let target_sector = (self.cursor / CD_FRAMEWORDS as i64) as Lsn;

        if target_sector > self.last_sector {
            return Err(Error::UnaddressableSector(target_sector));
        }

        // Report read start
        self.report_callback(self.cursor, ParanoiaCallback::Read);

        // Perform the read operation
        let success = if self.mode == ParanoiaMode::DISABLE {
            self.read_direct_internal(target_sector)
        } else {
            self.read_verified_internal(target_sector, max_retries)
        };

        if success {
            self.cursor += CD_FRAMEWORDS as i64;
            self.report_callback(self.cursor, ParanoiaCallback::Finished);
            Ok(&self.output_buffer)
        } else if self.enable_skip {
            self.report_callback(self.cursor, ParanoiaCallback::Skip);
            // Fill with silence on skip
            self.output_buffer.fill(0);
            self.cursor += CD_FRAMEWORDS as i64;
            Ok(&self.output_buffer)
        } else {
            Err(Error::SectorSkip(target_sector, max_retries))
        }
    }

    /// Direct read without verification (internal, returns success bool).
    fn read_direct_internal(&mut self, sector: Lsn) -> bool {
        let drive = unsafe { &mut *self.drive };
        match drive.read_audio(sector, 1) {
            Ok(data) => {
                let copy_len = data.len().min(CD_FRAMEWORDS);
                self.output_buffer[..copy_len].copy_from_slice(&data[..copy_len]);
                true
            }
            Err(_) => false,
        }
    }

    /// Read with full paranoia verification (internal, returns success bool).
    ///
    /// Uses the paranoia algorithm like libcdio-paranoia:
    /// 1. Check if data is already verified in root block
    /// 2. If not, read a large batch and verify against cached previous reads
    /// 3. For sequential reads, most data comes from cache (very efficient)
    #[allow(clippy::too_many_lines)]
    fn read_verified_internal(&mut self, target_sector: Lsn, max_retries: i32) -> bool {
        // Maximum number of backoffs before giving up on this sector
        const MAX_BACKOFFS: u32 = 3;

        let target_begin = target_sector as i64 * CD_FRAMEWORDS as i64;
        let target_end = target_begin + CD_FRAMEWORDS as i64;

        // Check if we already have this data verified in root
        if !self.root.is_empty() && self.root.begin <= target_begin && self.root.end() >= target_end
        {
            if let Some(frame) = self.root.extract_frame(target_begin) {
                self.output_buffer.copy_from_slice(frame);
                return true; // Fast path - return from cache
            }
        }

        let drive = unsafe { &mut *self.drive };

        // Read a large batch with backward overlap for verification
        // We need overlap with previous reads to verify continuity
        // Use dynamic overlap based on observed jitter
        let batch_size = 26i32;
        let dynamic_overlap = self.dynoverlap.overlap_sectors() as i32;
        let overlap_sectors = dynamic_overlap.max(2); // At least 2 sectors overlap
        let read_start = (target_sector - overlap_sectors).max(self.first_sector);
        let read_end = (read_start + batch_size + overlap_sectors).min(self.last_sector + 1);
        let read_count = (read_end - read_start) as i64;

        for retry in 0..max_retries {
            // Read the batch
            let new_data = if let Ok(data) = drive.read_audio(read_start, read_count) {
                // Record success
                self.backoff.record_success(target_sector);
                data
            } else {
                self.report_callback(target_begin, ParanoiaCallback::ReadError);

                // Record error and check if backoff is needed
                let needs_backoff = self.backoff.record_error(target_sector);
                let action = crate::backoff::determine_backoff_action(
                    &self.backoff,
                    needs_backoff,
                    MAX_BACKOFFS,
                );

                match action {
                    BackoffAction::Continue => {
                        // Not enough errors yet - just retry
                        continue;
                    }
                    BackoffAction::Backoff { sectors, delay } => {
                        // Perform backoff: seek away, wait, seek back
                        let backoff_target = (target_sector - sectors).max(self.first_sector);

                        // Seek away from problem area
                        let _ = drive.read_audio(backoff_target, 1);

                        // Wait for the drive to settle
                        std::thread::sleep(delay);

                        // Record that we performed a backoff
                        self.backoff.record_backoff();
                        self.report_callback(target_begin, ParanoiaCallback::Overlap);

                        continue;
                    }
                    BackoffAction::GiveUp => {
                        // Too many backoffs - give up on this sector
                        self.backoff.reset_error_state();
                        return false;
                    }
                }
            };

            let new_begin = read_start as i64 * CD_FRAMEWORDS as i64;

            // Stage 1: Try to verify against cached blocks
            // First try isort-based verification
            let (mut verified, jitter_offset) = self.verify_with_isort(&new_data, new_begin);

            if verified {
                // Found matching data in cache via isort - verified!
                if jitter_offset != 0 {
                    self.jitter = jitter_offset.abs();
                    self.record_jitter_with_report(jitter_offset, 1, new_begin);
                    self.report_callback(new_begin, ParanoiaCallback::FixupEdge);
                }
            } else if !self.cache.is_empty() {
                // Try overlap::find_overlap for more sophisticated matching
                let (overlap_found, overlap_offset) =
                    self.find_overlap_with_cache(&new_data, new_begin);
                if overlap_found {
                    verified = true;
                    if overlap_offset != 0 {
                        self.jitter = overlap_offset.abs();
                        self.record_jitter_with_report(overlap_offset, 1, new_begin);
                        self.report_callback(new_begin, ParanoiaCallback::FixupEdge);
                    }
                }
            }

            // If not verified against cache, do double-read verification
            if !verified {
                let Ok(read2) = drive.read_audio(read_start, read_count) else {
                    self.report_callback(target_begin, ParanoiaCallback::ReadError);

                    // Record error for backoff tracking
                    let needs_backoff = self.backoff.record_error(target_sector);
                    if needs_backoff && !self.backoff.should_give_up(MAX_BACKOFFS) {
                        let (sectors, delay) = self.backoff.get_backoff_params();
                        let backoff_target = (target_sector + sectors).max(self.first_sector);
                        let _ = drive.read_audio(backoff_target, 1);
                        std::thread::sleep(delay);
                        self.backoff.record_backoff();
                    }
                    continue;
                };

                if new_data == read2 {
                    // Perfect match - verified
                    verified = true;
                } else if self.find_batch_jitter_match(&new_data, &read2) {
                    // Jitter match found - verified
                    verified = true;
                } else {
                    // Double-read failed - try consensus voting with additional reads
                    self.report_callback(target_begin, ParanoiaCallback::Verify);

                    // Build consensus using multiple reads
                    let new_end = new_begin + new_data.len() as i64;
                    let mut builder = ConsensusBuilder::new(new_begin, new_end);
                    builder.add_read(ReadResult::new(new_data.clone(), new_begin, 0));
                    builder.add_read(ReadResult::new(read2, new_begin, 0));

                    // Do additional reads until we have enough for consensus
                    let search_range = self.dynoverlap.search;
                    let max_additional = MIN_READS_FOR_CONSENSUS + 2; // Up to 5 total reads
                    for i in 0..max_additional {
                        if builder.has_enough_reads() {
                            if let Some(result) = builder.build_with_stats() {
                                if result.is_acceptable() {
                                    break;
                                }
                            }
                        }

                        let Ok(additional) = drive.read_audio(read_start, read_count) else {
                            self.report_callback(new_begin, ParanoiaCallback::ReadError);
                            // Record error but don't do full backoff during consensus
                            self.backoff.record_error(target_sector);
                            continue;
                        };

                        // Detect jitter offset
                        let jitter =
                            detect_jitter_offset_static(&new_data, &additional, search_range);
                        builder.add_read(ReadResult::new(additional, new_begin, jitter));

                        if i == 0 {
                            self.report_callback(new_begin, ParanoiaCallback::Verify);
                        }
                    }

                    // Try to build final consensus
                    if let Some(result) = builder.build_with_stats() {
                        // Check acceptability before consuming result
                        let acceptable = result.is_acceptable();

                        let mut fragment =
                            VFragment::from_samples(&result.samples, new_begin, read_end as i64);
                        fragment.flags = result.flags;

                        // Add to cache
                        let block =
                            CBlock::from_sectors(&result.samples, new_begin, read_end as i64);
                        let sort = SortInfo::from_vector(&result.samples, new_begin);
                        self.cache.push(CachedBlock { block, sort });
                        self.trim_cache();

                        // Merge into root
                        self.try_merge_fragment(&fragment);
                        self.root.merge_fragment(&fragment);

                        if acceptable {
                            self.report_callback(target_begin, ParanoiaCallback::FixupAtom);
                        } else {
                            self.report_callback(target_begin, ParanoiaCallback::Scratch);
                        }

                        // Extract target frame from root
                        if let Some(frame) = self.root.extract_frame(target_begin) {
                            self.output_buffer.copy_from_slice(frame);
                            // Successfully read via consensus - reset backoff state
                            self.backoff.reset_error_state();
                            return true;
                        }
                    }
                    continue;
                }
            }

            // Create cache block with sort index
            let block = CBlock::from_sectors(&new_data, new_begin, read_end as i64);
            let sort = SortInfo::from_vector(&new_data, new_begin);
            self.cache.push(CachedBlock { block, sort });
            self.trim_cache();

            // Stage 2: Create verified fragment and merge into root
            // Fragment tracks per-sample verification status
            let mut fragment = if verified {
                // Data was verified against cache or double-read
                VFragment::from_samples_verified(&new_data, new_begin, read_end as i64)
            } else {
                // First read with no verification - mark as unverified
                VFragment::from_samples(&new_data, new_begin, read_end as i64)
            };

            // Mark edges as needing extra verification (first/last 588 samples = half frame)
            let edge_size = CD_FRAMEWORDS / 2;
            fragment.mark_edges(edge_size);

            // Add to fragments list for potential future merging
            self.try_merge_fragment(&fragment);

            // Merge fragment into root block using per-sample verification
            self.root.merge_fragment(&fragment);

            // Extract target frame from root
            if let Some(frame) = self.root.extract_frame(target_begin) {
                self.output_buffer.copy_from_slice(frame);
                // Successfully read - reset backoff state
                self.backoff.reset_error_state();
                if retry > 0 {
                    self.report_callback(target_begin, ParanoiaCallback::FixupEdge);
                }
                return true;
            }

            self.report_callback(target_begin, ParanoiaCallback::Verify);
        }

        // Exhausted retries - reset backoff and use last read anyway
        self.backoff.reset_error_state();
        if let Ok(data) = drive.read_audio(target_sector, 1) {
            let copy_len = data.len().min(CD_FRAMEWORDS);
            self.output_buffer[..copy_len].copy_from_slice(&data[..copy_len]);
            return true;
        }

        false
    }

    /// Check if two batch reads match, accounting for possible jitter.
    /// Uses silence detection and rift analysis when simple jitter matching fails.
    fn find_batch_jitter_match(&mut self, read1: &[i16], read2: &[i16]) -> bool {
        // Check at several positions in the batch for jitter
        let check_positions = [0, read1.len() / 4, read1.len() / 2, 3 * read1.len() / 4];
        let match_len = CD_FRAMEWORDS.min(read1.len());
        // Use dynamic search range based on observed jitter, minimum 32 samples
        let search_range = (self.dynoverlap.search as usize).max(32);

        for &pos in &check_positions {
            if pos + match_len > read1.len() {
                continue;
            }

            // Skip verification in silent regions - they all look the same
            if Self::is_region_silent(read1, pos, match_len) {
                // If both reads are silent at this position, check next position
                if Self::is_region_silent(read2, pos, match_len) {
                    continue;
                }
                // One is silent, one isn't - that's a problem, but check other positions
                continue;
            }

            let slice1 = &read1[pos..pos + match_len];

            // Try to find this slice in read2 with jitter offset
            for offset in 0..=search_range {
                for sign in &[0i32, 1, -1] {
                    let actual_offset = if *sign == 0 { 0 } else { offset as i32 * sign };
                    let pos2 = (pos as i32 + actual_offset) as usize;

                    if pos2 + match_len <= read2.len() && slice1 == &read2[pos2..pos2 + match_len] {
                        if actual_offset != 0 {
                            self.jitter = actual_offset.abs() as i64;
                            self.record_jitter_with_report(actual_offset as i64, 1, 0);
                            self.report_callback(0, ParanoiaCallback::FixupEdge);
                        }
                        return true;
                    }
                }
            }
        }

        // Jitter matching failed - try rift analysis
        if let Some(mismatch_pos) = Self::find_mismatch_position(read1, read2) {
            let rift = Self::analyze_rift(read1, read2, mismatch_pos);
            if let Some(_repaired) = self.handle_rift(read1, read2, rift) {
                // Rift was repaired - consider it a match
                return true;
            }
        }

        false
    }

    /// Verify new data against cached blocks using isort for efficient matching.
    /// Returns (verified, `jitter_offset`).
    fn verify_with_isort(&self, new_data: &[i16], new_begin: i64) -> (bool, i64) {
        if self.cache.is_empty() {
            return (false, 0);
        }

        // Try multiple positions in the new data for matching
        let try_positions = [
            0,
            new_data.len() / 4,
            new_data.len() / 2,
            3 * new_data.len() / 4,
        ];

        for cached in &self.cache {
            // Check if there's potential overlap
            let cached_end = cached.block.end();
            if new_begin >= cached_end || new_begin + new_data.len() as i64 <= cached.block.begin {
                continue; // No overlap
            }

            // Try to find matching runs using isort
            for &rel_pos in &try_positions {
                if rel_pos >= new_data.len() {
                    continue;
                }

                let abs_pos = new_begin + rel_pos as i64;

                // Use isort::find_match to find matching consecutive samples
                if let Some(offset) = isort::find_match(
                    new_data,
                    abs_pos,
                    new_begin,
                    &cached.sort,
                    MIN_WORDS_SEARCH as usize,
                ) {
                    // Found a match! Verify it's a substantial overlap
                    return (true, offset);
                }
            }
        }

        (false, 0)
    }

    /// Verify new data against cached blocks (Stage 1).
    /// Returns true if we find matching runs against any cached block.
    #[allow(dead_code)]
    fn verify_against_cache(&mut self, new_data: &[i16], new_begin: i64) -> bool {
        // Look for overlapping region with any cached block
        for cached in &self.cache {
            // Check if there's overlap between new data and this cached block
            let overlap_start = new_begin.max(cached.block.begin);
            let overlap_end = (new_begin + new_data.len() as i64).min(cached.block.end());

            if overlap_end > overlap_start {
                // There's overlap - compare the overlapping samples
                let new_offset = (overlap_start - new_begin) as usize;
                let cached_offset = (overlap_start - cached.block.begin) as usize;
                let overlap_len = (overlap_end - overlap_start) as usize;

                // Need at least MIN_WORDS_OVERLAP matching samples
                if overlap_len >= MIN_WORDS_OVERLAP as usize {
                    let new_slice = &new_data[new_offset..new_offset + overlap_len];
                    let cached_slice =
                        &cached.block.vector[cached_offset..cached_offset + overlap_len];

                    if new_slice == cached_slice {
                        // Found matching overlap - data is verified
                        return true;
                    }

                    // Try to find jittered match
                    if let Some(offset) =
                        self.find_jitter_match(new_slice, &cached.block.vector, cached_offset)
                    {
                        self.jitter = offset.abs();
                        self.dynoverlap.record_jitter(offset, 1);
                        self.report_callback(new_begin, ParanoiaCallback::FixupEdge);
                        return true;
                    }
                }
            }
        }

        // No cached data to verify against - trust first read
        self.cache.is_empty()
    }

    /// Find jitter offset between new samples and cached block.
    fn find_jitter_match(
        &self,
        new_slice: &[i16],
        cached_vec: &[i16],
        nominal_start: usize,
    ) -> Option<i64> {
        // Use dynamic search range, minimum 32 samples
        let search_range = self.dynoverlap.search.max(32);
        let match_len = new_slice.len().min(MIN_WORDS_OVERLAP as usize);

        // Start search from drift estimate (most likely offset)
        let drift = self.dynoverlap.drift;

        // Try drift offset first if non-zero
        if drift != 0 {
            let start = (nominal_start as i64 + drift) as usize;
            let end = start + match_len;
            if end <= cached_vec.len()
                && start < cached_vec.len()
                && new_slice[..match_len] == cached_vec[start..end]
            {
                return Some(drift);
            }
        }

        // Search around drift estimate
        for offset in 1..=search_range {
            for sign in &[1i64, -1i64] {
                let actual_offset = drift + offset * sign;
                let start = (nominal_start as i64 + actual_offset) as usize;
                let end = start + match_len;

                if end <= cached_vec.len()
                    && start < cached_vec.len()
                    && new_slice[..match_len] == cached_vec[start..end]
                {
                    return Some(actual_offset);
                }
            }
        }
        None
    }

    /// Check if a region is silent (near-zero samples).
    /// Silent regions can't be used for jitter detection because all silence looks the same.
    fn is_region_silent(data: &[i16], start: usize, len: usize) -> bool {
        if start + len > data.len() {
            return false;
        }
        gap::is_silent(&data[start..start + len], SILENCE_THRESHOLD)
    }

    /// Find the first non-silent position in data starting from given position.
    /// Returns None if the rest of the data is all silent.
    #[allow(dead_code)]
    fn find_non_silent(data: &[i16], start: usize) -> Option<usize> {
        for (i, &sample) in data[start..].iter().enumerate() {
            if sample.abs() > SILENCE_THRESHOLD {
                return Some(start + i);
            }
        }
        None
    }

    /// Analyze a rift (discontinuity) between two reads.
    /// Used when jitter matching fails to find simple offset.
    fn analyze_rift(read1: &[i16], read2: &[i16], mismatch_pos: usize) -> RiftType {
        // First check if the mismatch is in a silent region
        let check_len = MIN_WORDS_OVERLAP as usize;
        if mismatch_pos + check_len <= read1.len()
            && Self::is_region_silent(read1, mismatch_pos, check_len)
            && Self::is_region_silent(read2, mismatch_pos, check_len)
        {
            // Both are silent at mismatch point - can't determine rift, treat as OK
            return RiftType::NoRift;
        }

        // Use gap analysis to determine rift type
        let max_search = 256i64; // Search up to 256 samples for resync
        gap::analyze_rift_forward(
            read1,
            0,                   // offset_a
            mismatch_pos as i64, // end_a (position where mismatch starts)
            read2,
            0,                   // offset_b
            mismatch_pos as i64, // end_b
            max_search,
        )
    }

    /// Find first mismatch position between two reads.
    fn find_mismatch_position(read1: &[i16], read2: &[i16]) -> Option<usize> {
        let len = read1.len().min(read2.len());
        (0..len).find(|&i| read1[i] != read2[i])
    }

    /// Handle rift repair - attempt to reconcile dropped/duplicated samples.
    fn handle_rift(&mut self, read1: &[i16], read2: &[i16], rift: RiftType) -> Option<Vec<i16>> {
        match rift {
            RiftType::NoRift => Some(read1.to_vec()),

            RiftType::StutterInA(_count) => {
                // Block A has duplicate samples - use B's version
                // Or skip the stuttered samples in A
                self.report_callback(0, ParanoiaCallback::FixupDuped);
                Some(read2.to_vec())
            }

            RiftType::StutterInB(_count) => {
                // Block B has duplicate samples - use A's version
                self.report_callback(0, ParanoiaCallback::FixupDuped);
                Some(read1.to_vec())
            }

            RiftType::DroppedFromA(_count) => {
                // A is missing samples that B has - use B
                self.report_callback(0, ParanoiaCallback::FixupDropped);
                Some(read2.to_vec())
            }

            RiftType::DroppedFromB(_count) => {
                // B is missing samples that A has - use A
                self.report_callback(0, ParanoiaCallback::FixupDropped);
                Some(read1.to_vec())
            }

            RiftType::Garbage {
                sync_a: _,
                sync_b: _,
            } => {
                // Both have garbage - can't reliably repair
                self.report_callback(0, ParanoiaCallback::Scratch);
                None
            }

            RiftType::Unknown => {
                // Can't determine rift type
                None
            }
        }
    }

    /// Stage 1: Read data with overlap and create fragments.
    /// (Alternative implementation - kept for reference)
    #[allow(dead_code)]
    fn stage1_read(&mut self, target_sector: Lsn) -> Result<()> {
        // Calculate read range with overlap
        let overlap_sectors = self.dynoverlap.overlap_sectors() as i32;
        let read_start = (target_sector - overlap_sectors).max(self.first_sector);
        let read_end = (target_sector + overlap_sectors + 1).min(self.last_sector);
        let read_count = (read_end - read_start) as i64;

        // Read from drive
        let drive = unsafe { &mut *self.drive };
        let data = drive.read_audio(read_start, read_count)?;
        let begin_samples = read_start as i64 * CD_FRAMEWORDS as i64;

        // Create cache block with sort index
        let block = CBlock::from_sectors(&data, begin_samples, read_end as i64);
        let sort = SortInfo::from_vector(&data, begin_samples);

        // Add to cache
        self.cache.push(CachedBlock { block, sort });

        // Trim cache if too large
        self.trim_cache();

        // Report overlap adjustment if needed
        if self.cache.len() > 1 {
            self.report_callback(begin_samples, ParanoiaCallback::Overlap);
        }

        Ok(())
    }

    /// Stage 2: Verify fragments and merge into root.
    /// (Alternative implementation - kept for reference)
    #[allow(dead_code)]
    fn stage2_verify(&mut self) {
        if self.cache.is_empty() {
            return;
        }

        // If we only have one block, use it directly (no verification possible)
        if self.cache.len() == 1 {
            let cached = &self.cache[0];
            self.root.vector = cached.block.vector.clone();
            self.root.begin = cached.block.begin;
            self.root.lastsector = cached.block.lastsector;
            return;
        }

        // Use the latest block as base if root is empty
        if self.root.is_empty() {
            let latest = &self.cache[self.cache.len() - 1];
            self.root.vector = latest.block.vector.clone();
            self.root.begin = latest.block.begin;
            self.root.lastsector = latest.block.lastsector;
        }

        // Build sort index for root block
        let sort = SortInfo::from_vector(&self.root.vector, self.root.begin);

        // Compare cache blocks to find overlap and verify
        for i in 0..self.cache.len() {
            let current = &self.cache[i];

            // Skip if this block is already part of root
            if current.block.begin >= self.root.begin && current.block.end() <= self.root.end() {
                continue;
            }

            // Find overlap using sort index
            if let Some(offset) = crate::overlap::find_overlap(
                &current.block.vector,
                current.block.begin,
                &sort,
                MIN_WORDS_OVERLAP as usize,
                self.dynoverlap.search,
            ) {
                // Record jitter
                let jitter = offset.abs();
                if jitter > 0 {
                    self.dynoverlap.record_jitter(offset, 1);
                    self.jitter = jitter;

                    if offset != 0 {
                        let cb_begin = current.block.begin;
                        let event = if jitter < CD_FRAMEWORDS as i64 {
                            ParanoiaCallback::FixupEdge
                        } else {
                            ParanoiaCallback::FixupAtom
                        };
                        self.report_callback(cb_begin, event);
                    }
                }

                // Merge verified data
                self.merge_verified(i, offset);
            }
        }
    }

    /// Merge verified data into the root block.
    /// (Alternative implementation - kept for reference)
    #[allow(dead_code)]
    fn merge_verified(&mut self, cache_idx: usize, offset: i64) {
        let cached = &self.cache[cache_idx];

        // Simple merge: extend root with new verified data
        if self.root.is_empty() {
            self.root.vector = cached.block.vector.clone();
            self.root.begin = cached.block.begin;
            self.root.lastsector = cached.block.lastsector;
        } else {
            // Merge with existing root
            let new_begin = cached.block.begin + offset;
            let new_end = new_begin + cached.block.vector.len() as i64;

            if new_begin < self.root.begin {
                // Prepend
                let prepend_count = (self.root.begin - new_begin) as usize;
                let mut new_vec = cached.block.vector[..prepend_count].to_vec();
                new_vec.extend_from_slice(&self.root.vector);
                self.root.vector = new_vec;
                self.root.begin = new_begin;
            }

            if new_end > self.root.end() {
                // Append
                let start_idx = (self.root.end() - new_begin) as usize;
                if start_idx < cached.block.vector.len() {
                    self.root
                        .vector
                        .extend_from_slice(&cached.block.vector[start_idx..]);
                }
            }

            self.root.lastsector = self.root.lastsector.max(cached.block.lastsector);
        }
    }

    /// Trim the cache to stay within limits.
    fn trim_cache(&mut self) {
        let max_cache = (self.cache_model_sectors as usize).max(10);
        while self.cache.len() > max_cache {
            self.cache.remove(0);
        }
    }

    /// Report progress via callback.
    fn report_callback(&mut self, position: i64, event: ParanoiaCallback) {
        if let Some(ref mut cb) = self.callback {
            cb(position, event);
        }
    }

    /// Record jitter and check if overlap parameters changed.
    fn record_jitter_with_report(&mut self, offset: i64, stage: u8, position: i64) {
        let old_overlap = self.dynoverlap.overlap;
        self.dynoverlap.record_jitter(offset, stage);

        // Report if overlap was adjusted
        if self.dynoverlap.overlap != old_overlap {
            self.report_callback(position, ParanoiaCallback::Overlap);
        }
    }

    /// Try to find overlap between new data and cached blocks using `overlap::find_overlap`.
    /// Returns (found, offset) where offset is the jitter if found.
    fn find_overlap_with_cache(&self, new_data: &[i16], new_begin: i64) -> (bool, i64) {
        for cached in &self.cache {
            // Check if there's potential overlap
            let cached_end = cached.block.end();
            if new_begin >= cached_end || new_begin + new_data.len() as i64 <= cached.block.begin {
                continue; // No overlap possible
            }

            // Use overlap::find_overlap for sophisticated matching
            if let Some(offset) = overlap::find_overlap(
                new_data,
                new_begin,
                &cached.sort,
                MIN_WORDS_OVERLAP as usize,
                self.dynoverlap.search,
            ) {
                return (true, offset);
            }
        }
        (false, 0)
    }

    /// Try to merge a new fragment with existing fragments.
    /// Fragments that overlap and match are combined, marking overlapping regions as verified.
    fn try_merge_fragment(&mut self, new_fragment: &VFragment) {
        // Try to merge with existing fragments
        let mut merged = false;
        for existing in &mut self.fragments {
            if existing.try_merge(new_fragment) {
                merged = true;
                // Report successful merge
                self.report_callback(new_fragment.begin, ParanoiaCallback::FixupEdge);
                break;
            }
        }

        // If not merged, add as new fragment
        if !merged {
            self.fragments.push(new_fragment.clone());
        }

        // Consolidate fragments - merge any that now overlap
        self.consolidate_fragments();

        // Trim old fragments that are behind the cursor
        let cursor = self.cursor;
        self.fragments
            .retain(|f| f.end() > cursor - (CD_FRAMEWORDS * 10) as i64);
    }

    /// Consolidate fragments by merging overlapping ones.
    fn consolidate_fragments(&mut self) {
        if self.fragments.len() < 2 {
            return;
        }

        // Sort by begin position
        self.fragments.sort_by_key(|f| f.begin);

        // Merge adjacent/overlapping fragments
        let mut i = 0;
        while i + 1 < self.fragments.len() {
            // Try to merge fragment[i+1] into fragment[i]
            let should_merge = {
                let next = &self.fragments[i + 1];
                self.fragments[i].end() >= next.begin
            };

            if should_merge {
                let next = self.fragments.remove(i + 1);
                self.fragments[i].try_merge(&next);
                // Don't increment i - check if we can merge more
            } else {
                i += 1;
            }
        }
    }

    /// Get a reference to the underlying drive.
    ///
    /// # Safety
    /// The returned reference is only valid as long as the drive pointer is valid.
    #[must_use]
    pub fn drive(&self) -> &CdromDrive {
        unsafe { &*self.drive }
    }

    /// Get a mutable reference to the underlying drive.
    ///
    /// # Safety
    /// The returned reference is only valid as long as the drive pointer is valid.
    #[must_use]
    pub fn drive_mut(&mut self) -> &mut CdromDrive {
        unsafe { &mut *self.drive }
    }

    /// Get the current cursor position in samples.
    #[must_use]
    pub fn cursor(&self) -> i64 {
        self.cursor
    }

    /// Get the current cursor position in sectors.
    #[must_use]
    pub fn cursor_sector(&self) -> Lsn {
        (self.cursor / CD_FRAMEWORDS as i64) as Lsn
    }

    /// Get a reference to the output buffer.
    #[must_use]
    pub fn output_buffer(&self) -> &[i16] {
        &self.output_buffer
    }

    /// Check if the last returned frame was fully verified.
    #[must_use]
    pub fn last_frame_verified(&self) -> bool {
        let frame_begin = self.cursor - CD_FRAMEWORDS as i64;
        self.root.is_frame_verified(frame_begin)
    }

    /// Get the number of verified samples in the last returned frame.
    #[must_use]
    pub fn last_frame_verified_count(&self) -> usize {
        let frame_begin = self.cursor - CD_FRAMEWORDS as i64;
        self.root.frame_verified_count(frame_begin)
    }

    /// Get the verification percentage for the last returned frame.
    #[must_use]
    pub fn last_frame_verified_percent(&self) -> f64 {
        let count = self.last_frame_verified_count();
        (count as f64 / CD_FRAMEWORDS as f64) * 100.0
    }

    /// Get the number of fragments currently tracked.
    #[must_use]
    pub fn fragment_count(&self) -> usize {
        self.fragments.len()
    }

    /// Get total verified samples across all fragments.
    #[must_use]
    pub fn total_verified_samples(&self) -> usize {
        self.fragments
            .iter()
            .map(super::block::VFragment::verified_count)
            .sum()
    }
}

/// Iterator over paranoia-read sectors.
pub struct ParanoiaIter<'a> {
    paranoia: &'a mut Paranoia,
}

impl Iterator for ParanoiaIter<'_> {
    type Item = Result<Vec<i16>>;

    fn next(&mut self) -> Option<Self::Item> {
        let sector = self.paranoia.cursor_sector();
        if sector > self.paranoia.last_sector {
            return None;
        }

        Some(self.paranoia.read().map(<[i16]>::to_vec))
    }
}

impl Paranoia {
    /// Create an iterator over sectors.
    #[must_use]
    pub fn iter(&mut self) -> ParanoiaIter<'_> {
        ParanoiaIter { paranoia: self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paranoia_creation() {
        let drive = CdromDrive::new_stub();
        let paranoia = Paranoia::new(drive);
        assert_eq!(paranoia.mode(), ParanoiaMode::FULL);
    }

    #[test]
    fn test_paranoia_mode_set() {
        let drive = CdromDrive::new_stub();
        let mut paranoia = Paranoia::new(drive);

        paranoia.set_mode(ParanoiaMode::DISABLE);
        assert_eq!(paranoia.mode(), ParanoiaMode::DISABLE);

        paranoia.set_mode(ParanoiaMode::VERIFY | ParanoiaMode::OVERLAP);
        assert!(paranoia.mode().contains(ParanoiaMode::VERIFY));
        assert!(paranoia.mode().contains(ParanoiaMode::OVERLAP));
    }

    #[test]
    fn test_paranoia_seek() {
        let mut drive = CdromDrive::new_stub();
        drive.setup_test_disc(1, 0, 100);
        let mut paranoia = Paranoia::new(drive);

        let pos = paranoia.seek(50, SeekFrom::Start(0));
        assert_eq!(pos, 50);
        assert_eq!(paranoia.cursor_sector(), 50);
    }

    #[test]
    fn test_paranoia_read_disabled() {
        let mut drive = CdromDrive::new_stub();
        drive.setup_test_disc(1, 0, 10);

        // Load test data
        let test_data: Vec<i16> = (0..CD_FRAMEWORDS)
            .map(|value| i16::try_from(value).expect("CD_FRAMEWORDS fits in i16"))
            .collect();
        drive.load_test_sector(0, test_data.clone());

        let mut paranoia = Paranoia::new(drive);
        paranoia.set_mode(ParanoiaMode::DISABLE);

        let result = paranoia.read();
        assert!(result.is_ok());
        let data = result.unwrap();
        assert_eq!(data.len(), CD_FRAMEWORDS);
    }

    #[test]
    fn test_fragment_based_verification() {
        let mut drive = CdromDrive::new_stub();
        drive.setup_test_disc(1, 0, 50);

        // Load identical test data for multiple sectors (simulating a consistent disc)
        for sector in 0..50 {
            let test_data: Vec<i16> = (0..CD_FRAMEWORDS)
                .map(|i| ((sector * CD_FRAMEWORDS + i) % 32768) as i16)
                .collect();
            drive.load_test_sector(sector as i32, test_data);
        }

        let mut paranoia = Paranoia::new(drive);
        paranoia.set_mode(ParanoiaMode::FULL);

        // Read several sectors
        for _ in 0..5 {
            let result = paranoia.read();
            assert!(result.is_ok());
        }

        // Should have created fragments
        assert!(paranoia.fragment_count() > 0);
    }

    #[test]
    fn test_vfragment_merge() {
        use crate::block::VFragment;

        // Create two overlapping verified fragments
        let data1: Vec<i16> = (0..100).collect();
        let data2: Vec<i16> = (50..150).collect();

        let mut frag1 = VFragment::from_samples_verified(&data1, 0, 0);
        let frag2 = VFragment::from_samples_verified(&data2, 50, 0);

        // Merge should succeed (overlapping region 50-100 matches)
        assert!(frag1.try_merge(&frag2));

        // Result should span 0-150
        assert_eq!(frag1.begin, 0);
        assert_eq!(frag1.end(), 150);
        assert_eq!(frag1.len(), 150);

        // Overlapping region should be verified
        for i in 50..100 {
            assert!(frag1.is_verified(i), "Sample {i} should be verified");
        }
    }

    #[test]
    fn test_vfragment_merge_mismatch() {
        use crate::block::VFragment;

        // Create two fragments with different data in overlap region
        let data1: Vec<i16> = (0..100).collect();
        let mut data2: Vec<i16> = (50..150).collect();
        data2[0] = 9999; // Mismatch at position 50

        let mut frag1 = VFragment::from_samples_verified(&data1, 0, 0);
        let frag2 = VFragment::from_samples_verified(&data2, 50, 0);

        // Merge should fail (overlapping region doesn't match)
        assert!(!frag1.try_merge(&frag2));

        // Original fragment should be unchanged
        assert_eq!(frag1.len(), 100);
    }

    #[test]
    fn test_root_block_merge_fragment() {
        use crate::block::{RootBlock, VFragment};

        let mut root = RootBlock::new();

        // Create first fragment
        let data1: Vec<i16> = (0..100).collect();
        let frag1 = VFragment::from_samples_verified(&data1, 0, 0);
        root.merge_fragment(&frag1);

        assert_eq!(root.begin, 0);
        assert_eq!(root.end(), 100);

        // Merge overlapping fragment
        let data2: Vec<i16> = (50..200).collect();
        let frag2 = VFragment::from_samples_verified(&data2, 50, 0);
        root.merge_fragment(&frag2);

        // Root should now span 0-200
        assert_eq!(root.begin, 0);
        assert_eq!(root.end(), 200);

        // All samples should be verified (overlapping region marked verified)
        assert!(root.frame_verified_count(0) > 0);
    }
}
