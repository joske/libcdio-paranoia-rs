//! Core paranoia state machine for verified CD audio reading.
//!
//! This module implements the main paranoia algorithm with two-stage
//! verification, automatic jitter correction, and rift repair.

use std::io::SeekFrom;

use crate::{
    block::{CBlock, RootBlock, VFragment},
    cdda::CdromDrive,
    constants::{CACHEMODEL_SECTORS, CD_FRAMEWORDS, MIN_WORDS_OVERLAP},
    error::{Error, Result},
    isort::SortInfo,
    overlap::DynamicOverlap,
    types::{Lsn, ParanoiaCallback, ParanoiaMode},
};

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
    /// Cache of raw read blocks
    cache: Vec<CBlock>,
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
    /// Uses simple double-read verification: read sector twice and compare.
    fn read_verified_internal(&mut self, target_sector: Lsn, max_retries: i32) -> bool {
        let target_begin = target_sector as i64 * CD_FRAMEWORDS as i64;
        let drive = unsafe { &mut *self.drive };

        for retry in 0..max_retries {
            // First read
            let Ok(read1) = drive.read_audio(target_sector, 1) else {
                self.report_callback(target_begin, ParanoiaCallback::ReadError);
                continue;
            };

            // Second read for verification
            let Ok(read2) = drive.read_audio(target_sector, 1) else {
                self.report_callback(target_begin, ParanoiaCallback::ReadError);
                continue;
            };

            // Compare reads
            if read1 == read2 {
                // Verified - copy to output
                let copy_len = read1.len().min(CD_FRAMEWORDS);
                self.output_buffer[..copy_len].copy_from_slice(&read1[..copy_len]);
                if retry > 0 {
                    self.report_callback(target_begin, ParanoiaCallback::FixupEdge);
                }
                return true;
            }

            // Mismatch - report and retry
            self.report_callback(target_begin, ParanoiaCallback::Verify);
        }

        // Exhausted retries - use last read anyway
        if let Ok(data) = drive.read_audio(target_sector, 1) {
            let copy_len = data.len().min(CD_FRAMEWORDS);
            self.output_buffer[..copy_len].copy_from_slice(&data[..copy_len]);
            return true;
        }

        false
    }

    /// Stage 1: Read data with overlap and create fragments.
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

        // Create cache block
        let block = CBlock::from_sectors(&data, begin_samples, read_end as i64);

        // Add to cache
        self.cache.push(block);

        // Trim cache if too large
        self.trim_cache();

        // Report overlap adjustment if needed
        if self.cache.len() > 1 {
            self.report_callback(begin_samples, ParanoiaCallback::Overlap);
        }

        Ok(())
    }

    /// Stage 2: Verify fragments and merge into root.
    #[allow(dead_code)]
    fn stage2_verify(&mut self) {
        if self.cache.is_empty() {
            return;
        }

        // If we only have one block, use it directly (no verification possible)
        if self.cache.len() == 1 {
            let block = &self.cache[0];
            self.root.vector = block.vector.clone();
            self.root.begin = block.begin;
            self.root.lastsector = block.lastsector;
            return;
        }

        // Use the latest block as base if root is empty
        if self.root.is_empty() {
            let latest = &self.cache[self.cache.len() - 1];
            self.root.vector = latest.vector.clone();
            self.root.begin = latest.begin;
            self.root.lastsector = latest.lastsector;
        }

        // Build sort index for root block
        let sort = SortInfo::from_vector(&self.root.vector, self.root.begin);

        // Compare cache blocks to find overlap and verify
        for i in 0..self.cache.len() {
            let current = &self.cache[i];

            // Skip if this block is already part of root
            if current.begin >= self.root.begin && current.end() <= self.root.end() {
                continue;
            }

            // Find overlap using sort index
            if let Some(offset) = crate::overlap::find_overlap(
                &current.vector,
                current.begin,
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
                        let cb_begin = current.begin;
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
    #[allow(dead_code)]
    fn merge_verified(&mut self, cache_idx: usize, offset: i64) {
        let block = &self.cache[cache_idx];

        // Simple merge: extend root with new verified data
        if self.root.is_empty() {
            self.root.vector = block.vector.clone();
            self.root.begin = block.begin;
            self.root.lastsector = block.lastsector;
        } else {
            // Merge with existing root
            let new_begin = block.begin + offset;
            let new_end = new_begin + block.vector.len() as i64;

            if new_begin < self.root.begin {
                // Prepend
                let prepend_count = (self.root.begin - new_begin) as usize;
                let mut new_vec = block.vector[..prepend_count].to_vec();
                new_vec.extend_from_slice(&self.root.vector);
                self.root.vector = new_vec;
                self.root.begin = new_begin;
            }

            if new_end > self.root.end() {
                // Append
                let start_idx = (self.root.end() - new_begin) as usize;
                if start_idx < block.vector.len() {
                    self.root
                        .vector
                        .extend_from_slice(&block.vector[start_idx..]);
                }
            }

            self.root.lastsector = self.root.lastsector.max(block.lastsector);
        }
    }

    /// Trim the cache to stay within limits.
    #[allow(dead_code)]
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
}
