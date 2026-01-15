//! CDDA drive interface abstraction.
//!
//! This module provides an abstraction layer for CD-ROM drive access,
//! supporting both a stub implementation for testing and a real libcdio
//! backend when the `libcdio` feature is enabled.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::ptr;

use crate::constants::{CDIO_CD_FRAMESIZE_RAW, CD_FRAMEWORDS, MAXTRK};
use crate::error::{Error, Result, TransportError};
use crate::types::{Lsn, MessageDest, TocEntry, TrackNum};

#[cfg(feature = "libcdio")]
use libcdio_sys::{
    cdio_destroy, cdio_get_first_track_num, cdio_get_num_tracks, cdio_get_track_format,
    cdio_get_track_last_lsn, cdio_get_track_lsn, cdio_open, cdio_read_audio_sectors,
    driver_id_t_DRIVER_UNKNOWN, driver_return_code_t, driver_return_code_t_DRIVER_OP_SUCCESS,
    track_format_t_TRACK_FORMAT_AUDIO, CdIo_t, CDIO_INVALID_LSN,
};

/// CD-ROM drive handle.
///
/// Represents an opened CD-ROM drive with methods for reading audio data
/// and querying disc information.
pub struct CdromDrive {
    /// Device name/path
    pub device_name: Option<String>,
    /// Drive model string
    pub drive_model: Option<String>,
    /// Whether the drive is open
    pub opened: bool,
    /// Endianness: -1 = unknown, 0 = little, 1 = big
    pub bigendian: i32,
    /// Number of sectors to read at once
    pub nsectors: i32,
    /// Number of tracks on disc
    pub tracks: TrackNum,
    /// Table of contents
    pub disc_toc: [TocEntry; MAXTRK],
    /// First audio sector
    pub audio_first_sector: Lsn,
    /// Last audio sector
    pub audio_last_sector: Lsn,
    /// Whether to swap bytes
    pub swap_bytes: bool,
    /// Error destination
    pub error_dest: MessageDest,
    /// Message destination
    pub message_dest: MessageDest,
    /// Test flags for simulation
    pub test_flags: i32,
    /// Last read time in milliseconds
    pub last_milliseconds: i32,
    /// Track format flags (which tracks are audio)
    track_is_audio: [bool; MAXTRK],
    /// Backend implementation
    backend: DriveBackend,
}

// CdromDrive contains a raw pointer but is safe to use across threads
// when properly synchronized (the libcdio functions are thread-safe
// when called on different CdIo_t handles)
unsafe impl Send for CdromDrive {}

impl std::fmt::Debug for CdromDrive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdromDrive")
            .field("device_name", &self.device_name)
            .field("drive_model", &self.drive_model)
            .field("opened", &self.opened)
            .field("tracks", &self.tracks)
            .field("audio_first_sector", &self.audio_first_sector)
            .field("audio_last_sector", &self.audio_last_sector)
            .finish_non_exhaustive()
    }
}

/// Backend implementation for drive operations.
enum DriveBackend {
    /// Stub backend for testing
    Stub(StubBackend),
    /// Real libcdio backend
    #[cfg(feature = "libcdio")]
    Libcdio(LibcdioBackend),
}

/// Stub backend for testing without real hardware.
struct StubBackend {
    /// Simulated audio data (sector -> data)
    sectors: HashMap<Lsn, Vec<i16>>,
}

/// Real libcdio backend for CD drive access.
#[cfg(feature = "libcdio")]
struct LibcdioBackend {
    /// libcdio handle
    p_cdio: *mut CdIo_t,
}

#[cfg(feature = "libcdio")]
impl Drop for LibcdioBackend {
    fn drop(&mut self) {
        if !self.p_cdio.is_null() {
            unsafe {
                cdio_destroy(self.p_cdio);
            }
        }
    }
}

impl CdromDrive {
    /// Create a new stub drive for testing.
    pub fn new_stub() -> Self {
        Self {
            device_name: Some("stub".to_string()),
            drive_model: Some("Stub Drive".to_string()),
            opened: false,
            bigendian: 0,
            nsectors: 13,
            tracks: 0,
            disc_toc: [TocEntry::default(); MAXTRK],
            audio_first_sector: 0,
            audio_last_sector: 0,
            swap_bytes: false,
            error_dest: MessageDest::ForgetIt,
            message_dest: MessageDest::ForgetIt,
            test_flags: 0,
            last_milliseconds: 0,
            track_is_audio: [false; MAXTRK],
            backend: DriveBackend::Stub(StubBackend {
                sectors: HashMap::new(),
            }),
        }
    }

    /// Identify a CD-ROM drive by device path (does NOT open it).
    ///
    /// If `device` is None or empty, the default CD-ROM device is used.
    /// Call `open_drive()` after this to actually open the drive.
    #[cfg(feature = "libcdio")]
    pub fn identify_device(device: Option<&str>) -> Result<Self> {
        let device_cstr = device
            .filter(|s| !s.is_empty())
            .map(|s| CString::new(s).map_err(|_| Error::IdentifyError))
            .transpose()?;

        let device_ptr = device_cstr
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(ptr::null());

        let p_cdio = unsafe { cdio_open(device_ptr, driver_id_t_DRIVER_UNKNOWN) };

        if p_cdio.is_null() {
            return Err(Error::IdentifyError);
        }

        Ok(Self {
            device_name: device.map(String::from),
            drive_model: None,
            opened: false,
            bigendian: 0,
            nsectors: 13,
            tracks: 0,
            disc_toc: [TocEntry::default(); MAXTRK],
            audio_first_sector: 0,
            audio_last_sector: 0,
            swap_bytes: false,
            error_dest: MessageDest::ForgetIt,
            message_dest: MessageDest::ForgetIt,
            test_flags: 0,
            last_milliseconds: 0,
            track_is_audio: [false; MAXTRK],
            backend: DriveBackend::Libcdio(LibcdioBackend { p_cdio }),
        })
    }

    /// Identify a CD-ROM drive - stub version when libcdio is not available.
    #[cfg(not(feature = "libcdio"))]
    pub fn identify_device(_device: Option<&str>) -> Result<Self> {
        Err(Error::InterfaceNotSupported)
    }

    /// Open a CD-ROM drive by device path (identifies AND opens).
    ///
    /// If `device` is None or empty, the default CD-ROM device is used.
    #[cfg(feature = "libcdio")]
    pub fn open(device: Option<&str>) -> Result<Self> {
        let mut drive = Self::identify_device(device)?;
        drive.open_drive()?;
        Ok(drive)
    }

    /// Open a CD-ROM drive - stub version when libcdio is not available.
    #[cfg(not(feature = "libcdio"))]
    pub fn open(_device: Option<&str>) -> Result<Self> {
        Err(Error::InterfaceNotSupported)
    }

    /// Identify a CD-ROM drive by path (legacy API).
    ///
    /// Creates a drive handle without fully opening it.
    pub fn identify(device: &str, message_dest: MessageDest) -> Result<Self> {
        #[cfg(feature = "libcdio")]
        {
            let mut drive = Self::identify_device(Some(device))?;
            drive.message_dest = message_dest;
            Ok(drive)
        }

        #[cfg(not(feature = "libcdio"))]
        {
            let mut drive = Self::new_stub();
            drive.device_name = Some(device.to_string());
            drive.message_dest = message_dest;
            Ok(drive)
        }
    }

    /// Open the drive for reading.
    pub fn open_drive(&mut self) -> Result<()> {
        if self.opened {
            return Ok(());
        }

        match &self.backend {
            DriveBackend::Stub(_) => {
                self.opened = true;
                Ok(())
            }
            #[cfg(feature = "libcdio")]
            DriveBackend::Libcdio(_) => {
                self.read_toc()?;
                self.opened = true;
                Ok(())
            }
        }
    }

    /// Close the drive.
    pub fn close(&mut self) {
        self.opened = false;
        // The backend handles cleanup in its Drop implementation
    }

    /// Read the table of contents from the disc.
    #[cfg(feature = "libcdio")]
    fn read_toc(&mut self) -> Result<()> {
        let p_cdio = match &self.backend {
            DriveBackend::Libcdio(backend) => backend.p_cdio,
            _ => return Ok(()),
        };

        // Get number of tracks
        let num_tracks = unsafe { cdio_get_num_tracks(p_cdio) };
        if num_tracks == 0 || num_tracks == 255 {
            return Err(Error::NoAudioTracks);
        }
        self.tracks = num_tracks;

        // Get first track number
        let first_track = unsafe { cdio_get_first_track_num(p_cdio) };
        if first_track == 255 {
            return Err(Error::TocReadError("Invalid first track".to_string()));
        }

        // Read track information
        let mut first_audio_sector: Option<Lsn> = None;
        let mut last_audio_sector: Lsn = 0;

        for i in 0..num_tracks as usize {
            let track_num = first_track + i as u8;
            let lsn = unsafe { cdio_get_track_lsn(p_cdio, track_num) };

            if lsn == CDIO_INVALID_LSN {
                return Err(Error::TocReadError(format!(
                    "Invalid LSN for track {}",
                    track_num
                )));
            }

            self.disc_toc[i] = TocEntry {
                track: track_num,
                start_sector: lsn,
            };

            // Check if this track is audio
            let format = unsafe { cdio_get_track_format(p_cdio, track_num) };
            let is_audio = format == track_format_t_TRACK_FORMAT_AUDIO;
            self.track_is_audio[i] = is_audio;

            if is_audio {
                if first_audio_sector.is_none() {
                    first_audio_sector = Some(lsn);
                }

                // Get last sector of this track
                let last_lsn = unsafe { cdio_get_track_last_lsn(p_cdio, track_num) };
                if last_lsn != CDIO_INVALID_LSN && last_lsn > last_audio_sector {
                    last_audio_sector = last_lsn;
                }
            }
        }

        self.audio_first_sector = first_audio_sector.unwrap_or(0);
        self.audio_last_sector = last_audio_sector;

        if first_audio_sector.is_none() {
            return Err(Error::NoAudioTracks);
        }

        Ok(())
    }

    #[cfg(not(feature = "libcdio"))]
    fn read_toc(&mut self) -> Result<()> {
        Ok(())
    }

    /// Read audio sectors from the disc.
    ///
    /// Reads `sectors` sectors starting at `begin_sector`.
    /// Returns a vector of 16-bit audio samples.
    pub fn read_audio(&mut self, begin_sector: Lsn, sectors: i64) -> Result<Vec<i16>> {
        if !self.opened {
            return Err(Error::DeviceNotOpen);
        }

        let total_samples = sectors as usize * CD_FRAMEWORDS;

        match &self.backend {
            DriveBackend::Stub(stub) => {
                let mut buffer = vec![0i16; total_samples];
                // Return any pre-loaded test data, or zeros
                for s in 0..sectors {
                    let sector = begin_sector + s as i32;
                    let offset = s as usize * CD_FRAMEWORDS;

                    if let Some(data) = stub.sectors.get(&sector) {
                        let copy_len = data.len().min(CD_FRAMEWORDS);
                        buffer[offset..offset + copy_len].copy_from_slice(&data[..copy_len]);
                    }
                }
                self.last_milliseconds = 10; // Simulated read time
                Ok(buffer)
            }

            #[cfg(feature = "libcdio")]
            DriveBackend::Libcdio(backend) => {
                let bytes_needed = sectors as usize * CDIO_CD_FRAMESIZE_RAW;
                let mut buffer = vec![0u8; bytes_needed];

                let start_time = std::time::Instant::now();

                let result = unsafe {
                    cdio_read_audio_sectors(
                        backend.p_cdio,
                        buffer.as_mut_ptr() as *mut _,
                        begin_sector,
                        sectors as u32,
                    )
                };

                self.last_milliseconds = start_time.elapsed().as_millis() as i32;

                if result != driver_return_code_t_DRIVER_OP_SUCCESS {
                    return Err(Error::TransportError(Self::driver_code_to_error(result)));
                }

                // Convert bytes to i16 samples
                // CD audio is 16-bit little-endian stereo
                let samples: Vec<i16> = buffer
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();

                // Apply byte swapping if needed
                if self.swap_bytes {
                    Ok(samples.into_iter().map(|s| s.swap_bytes()).collect())
                } else {
                    Ok(samples)
                }
            }
        }
    }

    /// Convert libcdio driver return code to our error type.
    #[cfg(feature = "libcdio")]
    fn driver_code_to_error(code: driver_return_code_t) -> TransportError {
        match code {
            0 => TransportError::Ok,
            1 => TransportError::WriteError,
            2 => TransportError::ReadError,
            3 => TransportError::Underrun,
            4 => TransportError::Overrun,
            5 => TransportError::Illegal,
            6 => TransportError::Medium,
            7 => TransportError::Busy,
            8 => TransportError::NotReady,
            9 => TransportError::Fault,
            _ => TransportError::Unknown,
        }
    }

    /// Read audio sectors with timing information.
    pub fn read_audio_timed(
        &mut self,
        begin_sector: Lsn,
        sectors: i64,
    ) -> Result<(Vec<i16>, i32)> {
        let data = self.read_audio(begin_sector, sectors)?;
        Ok((data, self.last_milliseconds))
    }

    /// Get the first sector of a track.
    pub fn track_first_sector(&self, track: TrackNum) -> Result<Lsn> {
        if track == 0 || track > self.tracks {
            return Err(Error::InvalidTrack(track));
        }
        Ok(self.disc_toc[track as usize - 1].start_sector)
    }

    /// Get the last sector of a track.
    pub fn track_last_sector(&self, track: TrackNum) -> Result<Lsn> {
        if track == 0 || track > self.tracks {
            return Err(Error::InvalidTrack(track));
        }

        #[cfg(feature = "libcdio")]
        if let DriveBackend::Libcdio(backend) = &self.backend {
            let lsn = unsafe { cdio_get_track_last_lsn(backend.p_cdio, track) };
            if lsn != CDIO_INVALID_LSN {
                return Ok(lsn);
            }
        }

        // Fallback: use next track start - 1, or audio_last_sector
        if track == self.tracks {
            Ok(self.audio_last_sector)
        } else {
            Ok(self.disc_toc[track as usize].start_sector - 1)
        }
    }

    /// Get the number of tracks.
    pub fn track_count(&self) -> TrackNum {
        self.tracks
    }

    /// Get the track containing a given sector.
    pub fn sector_get_track(&self, lsn: Lsn) -> Option<TrackNum> {
        #[cfg(feature = "libcdio")]
        if let DriveBackend::Libcdio(backend) = &self.backend {
            let track = unsafe { libcdio_sys::cdio_get_track(backend.p_cdio, lsn) };
            if track != 255 {
                return Some(track);
            }
        }

        // Fallback: search TOC
        for i in (0..self.tracks as usize).rev() {
            if lsn >= self.disc_toc[i].start_sector {
                return Some(self.disc_toc[i].track);
            }
        }
        None
    }

    /// Check if a track is audio.
    pub fn track_is_audio(&self, track: TrackNum) -> bool {
        if track == 0 || track > self.tracks {
            return false;
        }

        #[cfg(feature = "libcdio")]
        if let DriveBackend::Libcdio(backend) = &self.backend {
            let format = unsafe { cdio_get_track_format(backend.p_cdio, track) };
            return format == track_format_t_TRACK_FORMAT_AUDIO;
        }

        // Fallback to cached value or assume audio
        self.track_is_audio
            .get(track as usize - 1)
            .copied()
            .unwrap_or(true)
    }

    /// Get the number of channels for a track (2 for stereo, 4 for quad).
    pub fn track_channels(&self, track: TrackNum) -> i32 {
        #[cfg(feature = "libcdio")]
        if let DriveBackend::Libcdio(backend) = &self.backend {
            let channels =
                unsafe { libcdio_sys::cdio_get_track_channels(backend.p_cdio, track) };
            if channels > 0 {
                return channels as i32;
            }
        }

        2 // Default to stereo
    }

    /// Get first audio sector on disc.
    pub fn disc_first_sector(&self) -> Lsn {
        self.audio_first_sector
    }

    /// Get last audio sector on disc.
    pub fn disc_last_sector(&self) -> Lsn {
        self.audio_last_sector
    }

    /// Set drive speed.
    pub fn set_speed(&mut self, speed: i32) -> Result<()> {
        #[cfg(feature = "libcdio")]
        if let DriveBackend::Libcdio(backend) = &self.backend {
            let result =
                unsafe { libcdio_sys::cdio_set_speed(backend.p_cdio, speed) };
            if result != driver_return_code_t_DRIVER_OP_SUCCESS {
                return Err(Error::OptionNotSupported);
            }
        }

        Ok(())
    }

    /// Set verbose output options.
    pub fn set_verbose(&mut self, error_dest: MessageDest, message_dest: MessageDest) {
        self.error_dest = error_dest;
        self.message_dest = message_dest;
    }

    /// Detect byte order of audio data from the drive.
    ///
    /// Returns 1 if big-endian, 0 if little-endian, -1 if unknown.
    pub fn detect_endianness(&mut self) -> i32 {
        // For now, assume little-endian (most common)
        // TODO: Implement FFT-based detection from original code
        self.bigendian = 0;
        0
    }

    // =========================================================================
    // Test helpers
    // =========================================================================

    /// Load test data for a sector (stub backend only).
    #[cfg(test)]
    pub fn load_test_sector(&mut self, sector: Lsn, data: Vec<i16>) {
        if let DriveBackend::Stub(stub) = &mut self.backend {
            stub.sectors.insert(sector, data);
        }
    }

    /// Set up a test disc with the given parameters (stub backend only).
    #[cfg(test)]
    pub fn setup_test_disc(&mut self, tracks: TrackNum, first_sector: Lsn, last_sector: Lsn) {
        self.tracks = tracks;
        self.audio_first_sector = first_sector;
        self.audio_last_sector = last_sector;
        self.opened = true;

        // Set up a simple TOC
        let sectors_per_track = (last_sector - first_sector) / tracks as i32;
        for i in 0..tracks as usize {
            self.disc_toc[i] = TocEntry {
                track: (i + 1) as u8,
                start_sector: first_sector + (i as i32 * sectors_per_track),
            };
            self.track_is_audio[i] = true;
        }
    }
}

impl Drop for CdromDrive {
    fn drop(&mut self) {
        self.close();
    }
}

/// Get the default CD-ROM device path.
#[allow(dead_code)]
#[cfg(feature = "libcdio")]
pub fn get_default_device() -> Option<String> {
    let device_ptr = unsafe { libcdio_sys::cdio_get_default_device(ptr::null()) };
    if device_ptr.is_null() {
        return None;
    }

    let device = unsafe { CStr::from_ptr(device_ptr) }
        .to_str()
        .ok()
        .map(String::from);

    unsafe {
        libc::free(device_ptr as *mut _);
    }

    device
}

#[allow(dead_code)]
#[cfg(not(feature = "libcdio"))]
pub fn get_default_device() -> Option<String> {
    None
}

/// Version string for the CDDA interface.
#[allow(dead_code)]
pub fn version() -> &'static str {
    crate::constants::VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stub_drive_creation() {
        let drive = CdromDrive::new_stub();
        assert!(!drive.opened);
        assert_eq!(drive.device_name, Some("stub".to_string()));
    }

    #[test]
    fn test_stub_drive_open_close() {
        let mut drive = CdromDrive::new_stub();
        assert!(drive.open_drive().is_ok());
        assert!(drive.opened);
        drive.close();
        assert!(!drive.opened);
    }

    #[test]
    fn test_stub_drive_read() {
        let mut drive = CdromDrive::new_stub();
        drive.opened = true;

        // Load test data
        let test_data: Vec<i16> = (0..CD_FRAMEWORDS as i16).collect();
        drive.load_test_sector(100, test_data.clone());

        // Read it back
        let result = drive.read_audio(100, 1).unwrap();
        assert_eq!(result.len(), CD_FRAMEWORDS);
        assert_eq!(&result[..], &test_data[..]);
    }

    #[test]
    fn test_track_info() {
        let mut drive = CdromDrive::new_stub();
        drive.setup_test_disc(3, 0, 300);

        assert_eq!(drive.track_count(), 3);
        assert_eq!(drive.track_first_sector(1).unwrap(), 0);
        assert_eq!(drive.track_first_sector(2).unwrap(), 100);
    }

    #[test]
    #[cfg(feature = "libcdio")]
    fn test_get_default_device() {
        // This may or may not return a device depending on the system
        let _ = get_default_device();
    }
}
