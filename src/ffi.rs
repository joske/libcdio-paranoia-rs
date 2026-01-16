//! C FFI exports for ABI compatibility.
//!
//! This module provides C-compatible function exports that match the
//! original libcdio-paranoia API, allowing this library to be used as
//! a drop-in replacement for the C library.

use std::{
    ffi::{c_char, c_int, c_long, c_void, CStr, CString},
    io::SeekFrom,
    ptr,
};

use libc::c_short;

use crate::{
    cdda::CdromDrive,
    constants::MAXTRK,
    paranoia::Paranoia,
    types::{Lsn, MessageDest, ParanoiaMode},
};

/// Opaque paranoia handle for C API.
pub type CdromParanoiaT = Paranoia;

/// Opaque drive handle for C API.
pub type CdromDriveT = CdromDrive;

/// C-compatible callback function type.
pub type ParanoiaCallbackT = Option<extern "C" fn(c_long, c_int)>;

/// C-compatible TOC structure.
#[repr(C)]
pub struct TocT {
    pub b_track: u8,
    pub dw_start_sector: i32,
}

/// C-compatible drive structure (matches `cdrom_drive_s` layout).
#[repr(C)]
pub struct CdromDriveS {
    pub p_cdio: *mut c_void,
    pub opened: c_int,
    pub cdda_device_name: *mut c_char,
    pub drive_model: *mut c_char,
    pub drive_type: c_int,
    pub bigendianp: c_int,
    pub nsectors: c_int,
    pub cd_extra: c_int,
    pub b_swap_bytes: c_int,
    pub tracks: u8,
    pub disc_toc: [TocT; MAXTRK],
    pub audio_first_sector: Lsn,
    pub audio_last_sector: Lsn,
    pub errordest: c_int,
    pub messagedest: c_int,
    pub errorbuf: *mut c_char,
    pub messagebuf: *mut c_char,
    // Function pointers omitted - not directly usable from Rust anyway
}

// ============================================================================
// Version functions
// ============================================================================

/// Wrapper to make raw pointers Sync for static storage.
#[repr(transparent)]
pub struct SyncPtr(pub *const c_char);
unsafe impl Sync for SyncPtr {}

/// Version string (null-terminated).
static VERSION_STR: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();

/// Get libcdio-paranoia version string.
#[no_mangle]
pub extern "C" fn cdio_paranoia_version() -> *const c_char {
    VERSION_STR.as_ptr().cast()
}

/// Get CDDA interface version string.
#[no_mangle]
pub extern "C" fn cdio_cddap_version() -> *const c_char {
    cdio_paranoia_version()
}

// ============================================================================
// Paranoia functions
// ============================================================================

/// Initialize paranoia from a drive handle.
///
/// Note: paranoia does NOT take ownership of the drive.
/// The caller must keep the drive alive while paranoia is in use,
/// and must free the drive separately after freeing paranoia.
#[no_mangle]
pub unsafe extern "C" fn cdio_paranoia_init(d: *mut CdromDriveT) -> *mut CdromParanoiaT {
    if d.is_null() {
        return ptr::null_mut();
    }

    // Create paranoia from drive pointer (does NOT take ownership)
    let paranoia = unsafe { Paranoia::from_drive_ptr(d) };
    Box::into_raw(Box::new(paranoia))
}

/// Free paranoia resources.
#[no_mangle]
pub unsafe extern "C" fn cdio_paranoia_free(p: *mut CdromParanoiaT) {
    if !p.is_null() {
        drop(unsafe { Box::from_raw(p) });
    }
}

/// Set paranoia mode.
#[no_mangle]
pub unsafe extern "C" fn cdio_paranoia_modeset(p: *mut CdromParanoiaT, mode_flags: c_int) {
    if let Some(paranoia) = unsafe { p.as_mut() } {
        paranoia.set_mode(ParanoiaMode::from_bits_truncate(mode_flags));
    }
}

/// Seek to a position.
#[no_mangle]
pub unsafe extern "C" fn cdio_paranoia_seek(
    p: *mut CdromParanoiaT,
    seek: i32,
    whence: c_int,
) -> Lsn {
    let Some(paranoia) = (unsafe { p.as_mut() }) else {
        return -1;
    };

    let seek_from = match whence {
        0 => SeekFrom::Start(0),   // SEEK_SET
        1 => SeekFrom::Current(0), // SEEK_CUR
        2 => SeekFrom::End(0),     // SEEK_END
        _ => return -1,
    };

    paranoia.seek(seek, seek_from)
}

/// Read next sector with verification.
#[no_mangle]
pub unsafe extern "C" fn cdio_paranoia_read(
    p: *mut CdromParanoiaT,
    callback: ParanoiaCallbackT,
) -> *mut c_short {
    cdio_paranoia_read_limited(p, callback, 20)
}

/// Read next sector with verification and retry limit.
#[no_mangle]
pub unsafe extern "C" fn cdio_paranoia_read_limited(
    p: *mut CdromParanoiaT,
    callback: ParanoiaCallbackT,
    max_retries: c_int,
) -> *mut c_short {
    let Some(paranoia) = (unsafe { p.as_mut() }) else {
        return ptr::null_mut();
    };

    // Set up callback wrapper if provided
    if let Some(cb) = callback {
        paranoia.set_callback(move |pos, event| {
            cb(pos as c_long, event as c_int);
        });
    } else {
        paranoia.clear_callback();
    }

    match paranoia.read_limited(max_retries) {
        Ok(data) => {
            // The data is stored in paranoia's internal buffer
            // Return pointer to it (valid until next read)
            data.as_ptr().cast_mut()
        }
        Err(_) => ptr::null_mut(),
    }
}

/// Set overlap (temporary hack from original API).
#[no_mangle]
pub unsafe extern "C" fn cdio_paranoia_overlapset(p: *mut CdromParanoiaT, overlap: c_long) {
    if let Some(paranoia) = unsafe { p.as_mut() } {
        paranoia.set_overlap(overlap);
    }
}

/// Set reading range.
#[no_mangle]
pub unsafe extern "C" fn cdio_paranoia_set_range(
    p: *mut CdromParanoiaT,
    start: c_long,
    end: c_long,
) {
    if let Some(paranoia) = unsafe { p.as_mut() } {
        paranoia.set_range(start as Lsn, end as Lsn);
    }
}

/// Set or query cache model size.
#[no_mangle]
pub unsafe extern "C" fn cdio_paranoia_cachemodel_size(
    p: *mut CdromParanoiaT,
    sectors: c_int,
) -> c_int {
    let Some(paranoia) = (unsafe { p.as_mut() }) else {
        return -1;
    };
    paranoia.cache_model_size(sectors)
}

// ============================================================================
// CDDA drive functions
// ============================================================================

/// Find a CD-ROM drive with audio disc.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_find_a_cdrom(
    _messagedest: c_int,
    ppsz_message: *mut *mut c_char,
) -> *mut CdromDriveT {
    // Stub implementation - would need full libcdio integration
    if !ppsz_message.is_null() {
        unsafe { *ppsz_message = ptr::null_mut() };
    }
    ptr::null_mut()
}

/// Identify a CD-ROM drive by path (does NOT open it).
/// Call `cdio_cddap_open()` after this to open the drive.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_identify(
    psz_device: *const c_char,
    messagedest: c_int,
    ppsz_message: *mut *mut c_char,
) -> *mut CdromDriveT {
    // Get device string, or None for default
    let device: Option<&str> = if psz_device.is_null() {
        None
    } else {
        match unsafe { CStr::from_ptr(psz_device) }.to_str() {
            Ok(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    };

    let destination = MessageDest::from(messagedest);

    if !ppsz_message.is_null() {
        unsafe { *ppsz_message = ptr::null_mut() };
    }

    match CdromDrive::identify_device(device) {
        Ok(mut drive) => {
            drive.message_dest = destination;
            Box::into_raw(Box::new(drive))
        }
        Err(_) => ptr::null_mut(),
    }
}

/// Identify using existing libcdio handle.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_identify_cdio(
    _p_cdio: *mut c_void,
    _messagedest: c_int,
    ppsz_messages: *mut *mut c_char,
) -> *mut CdromDriveT {
    // Would require libcdio integration
    if !ppsz_messages.is_null() {
        unsafe { *ppsz_messages = ptr::null_mut() };
    }
    ptr::null_mut()
}

/// Open the drive.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_open(d: *mut CdromDriveT) -> c_int {
    let Some(drive) = (unsafe { d.as_mut() }) else {
        return -1;
    };
    match drive.open_drive() {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Close the drive.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_close(d: *mut CdromDriveT) -> c_int {
    if d.is_null() {
        return 0;
    }
    let drive = unsafe { Box::from_raw(d) };
    drop(drive);
    1
}

/// Close drive without freeing libcdio handle.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_close_no_free_cdio(d: *mut CdromDriveT) -> c_int {
    if d.is_null() {
        return 0;
    }
    let mut drive = unsafe { Box::from_raw(d) };
    drive.close();
    drop(drive);
    1
}

/// Read audio sectors.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_read(
    d: *mut CdromDriveT,
    p_buffer: *mut c_void,
    beginsector: Lsn,
    sectors: c_long,
) -> c_long {
    let Some(drive) = (unsafe { d.as_mut() }) else {
        return -1;
    };

    match drive.read_audio(beginsector, sectors) {
        Ok(data) => {
            if !p_buffer.is_null() {
                let dst = p_buffer.cast::<i16>();
                unsafe { ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len()) };
            }
            sectors
        }
        Err(_) => -1,
    }
}

/// Read audio sectors with timing.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_read_timed(
    d: *mut CdromDriveT,
    p_buffer: *mut c_void,
    beginsector: Lsn,
    sectors: c_long,
    milliseconds: *mut c_int,
) -> c_long {
    let Some(drive) = (unsafe { d.as_mut() }) else {
        return -1;
    };

    match drive.read_audio_timed(beginsector, sectors) {
        Ok((data, ms)) => {
            if !p_buffer.is_null() {
                let dst = p_buffer.cast::<i16>();
                unsafe { ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len()) };
            }
            if !milliseconds.is_null() {
                unsafe { *milliseconds = ms };
            }
            sectors
        }
        Err(_) => -1,
    }
}

/// Get first sector of a track.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_track_firstsector(d: *mut CdromDriveT, i_track: u8) -> Lsn {
    let Some(drive) = (unsafe { d.as_ref() }) else {
        return -1;
    };
    drive.track_first_sector(i_track).unwrap_or(-1)
}

/// Get last sector of a track.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_track_lastsector(d: *mut CdromDriveT, i_track: u8) -> Lsn {
    let Some(drive) = (unsafe { d.as_ref() }) else {
        return -1;
    };
    drive.track_last_sector(i_track).unwrap_or(-1)
}

/// Get number of tracks.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_tracks(d: *mut CdromDriveT) -> c_int {
    unsafe { d.as_ref() }.map_or(0, |d| d.track_count() as c_int)
}

/// Get track containing a sector.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_sector_gettrack(d: *mut CdromDriveT, lsn: Lsn) -> c_int {
    let Some(drive) = (unsafe { d.as_ref() }) else {
        return -1;
    };
    drive.sector_get_track(lsn).map_or(-1, c_int::from)
}

/// Get track channel count.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_track_channels(d: *mut CdromDriveT, i_track: u8) -> c_int {
    let Some(drive) = (unsafe { d.as_ref() }) else {
        return 2; // Default to stereo
    };
    drive.track_channels(i_track)
}

/// Check if track is audio.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_track_audiop(d: *mut CdromDriveT, i_track: u8) -> c_int {
    let Some(drive) = (unsafe { d.as_ref() }) else {
        return 0;
    };
    i32::from(drive.track_is_audio(i_track))
}

/// Check if track has copy permit.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_track_copyp(d: *mut CdromDriveT, i_track: u8) -> c_int {
    let Some(drive) = (unsafe { d.as_ref() }) else {
        return 0;
    };
    drive.track_copyp(i_track)
}

/// Check if track has preemphasis.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_track_preemp(d: *mut CdromDriveT, i_track: u8) -> c_int {
    let Some(drive) = (unsafe { d.as_ref() }) else {
        return 0;
    };
    drive.track_preemp(i_track)
}

/// Get first audio sector on disc.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_disc_firstsector(d: *mut CdromDriveT) -> Lsn {
    unsafe { d.as_ref() }.map_or(-1, CdromDrive::disc_first_sector)
}

/// Get last audio sector on disc.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_disc_lastsector(d: *mut CdromDriveT) -> Lsn {
    unsafe { d.as_ref() }.map_or(-1, CdromDrive::disc_last_sector)
}

/// Set drive speed.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_speed_set(d: *mut CdromDriveT, speed: c_int) -> c_int {
    let Some(drive) = (unsafe { d.as_mut() }) else {
        return -1;
    };
    match drive.set_speed(speed) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Set verbose output options.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_verbose_set(
    d: *mut CdromDriveT,
    err_action: c_int,
    mes_action: c_int,
) {
    if let Some(drive) = unsafe { d.as_mut() } {
        let error_dest = MessageDest::from(err_action);
        let message_dest = MessageDest::from(mes_action);
        drive.set_verbose(error_dest, message_dest);
    }
}

/// Get error messages.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_errors(_d: *mut CdromDriveT) -> *mut c_char {
    ptr::null_mut() // Stub
}

/// Get messages.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_messages(_d: *mut CdromDriveT) -> *mut c_char {
    ptr::null_mut() // Stub
}

/// Free message buffer.
#[no_mangle]
pub unsafe extern "C" fn cdio_cddap_free_messages(psz_messages: *mut c_char) {
    if !psz_messages.is_null() {
        drop(unsafe { CString::from_raw(psz_messages) });
    }
}

/// Detect endianness of CD data.
///
/// Returns 1 if big-endian, 0 if little-endian, -1 if unknown.
#[no_mangle]
pub unsafe extern "C" fn data_bigendianp(d: *mut CdromDriveT) -> c_int {
    let Some(drive) = (unsafe { d.as_ref() }) else {
        return -1; // Unknown
    };
    drive.bigendian
}

// ============================================================================
// Callback mode string array (for debugging)
// ============================================================================

/// Callback mode names for debugging.
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static paranoia_cb_mode2str: [SyncPtr; 16] = [
    SyncPtr(c"read".as_ptr()),
    SyncPtr(c"verify".as_ptr()),
    SyncPtr(c"fixup_edge".as_ptr()),
    SyncPtr(c"fixup_atom".as_ptr()),
    SyncPtr(c"scratch".as_ptr()),
    SyncPtr(c"repair".as_ptr()),
    SyncPtr(c"skip".as_ptr()),
    SyncPtr(c"drift".as_ptr()),
    SyncPtr(c"backoff".as_ptr()),
    SyncPtr(c"overlap".as_ptr()),
    SyncPtr(c"fixup_dropped".as_ptr()),
    SyncPtr(c"fixup_duped".as_ptr()),
    SyncPtr(c"readerr".as_ptr()),
    SyncPtr(c"cacheerr".as_ptr()),
    SyncPtr(c"wrote".as_ptr()),
    SyncPtr(c"finished".as_ptr()),
];

// ============================================================================
// Compatibility aliases (old paranoia API)
// ============================================================================

// These are provided for compatibility with code using the old paranoia API
// without the cdio_ prefix.

#[no_mangle]
pub extern "C" fn paranoia_version() -> *const c_char {
    cdio_paranoia_version()
}

#[no_mangle]
pub unsafe extern "C" fn paranoia_init(d: *mut CdromDriveT) -> *mut CdromParanoiaT {
    unsafe { cdio_paranoia_init(d) }
}

#[no_mangle]
pub unsafe extern "C" fn paranoia_free(p: *mut CdromParanoiaT) {
    unsafe { cdio_paranoia_free(p) }
}

#[no_mangle]
pub unsafe extern "C" fn paranoia_modeset(p: *mut CdromParanoiaT, mode_flags: c_int) {
    unsafe { cdio_paranoia_modeset(p, mode_flags) }
}

#[no_mangle]
pub unsafe extern "C" fn paranoia_seek(p: *mut CdromParanoiaT, seek: i32, whence: c_int) -> Lsn {
    unsafe { cdio_paranoia_seek(p, seek, whence) }
}

#[no_mangle]
pub unsafe extern "C" fn paranoia_read(
    p: *mut CdromParanoiaT,
    callback: ParanoiaCallbackT,
) -> *mut c_short {
    unsafe { cdio_paranoia_read(p, callback) }
}

#[no_mangle]
pub unsafe extern "C" fn paranoia_read_limited(
    p: *mut CdromParanoiaT,
    callback: ParanoiaCallbackT,
    max_retries: c_int,
) -> *mut c_short {
    unsafe { cdio_paranoia_read_limited(p, callback, max_retries) }
}

#[no_mangle]
pub unsafe extern "C" fn paranoia_overlapset(p: *mut CdromParanoiaT, overlap: c_long) {
    unsafe { cdio_paranoia_overlapset(p, overlap) }
}

#[no_mangle]
pub unsafe extern "C" fn paranoia_set_range(p: *mut CdromParanoiaT, start: c_long, end: c_long) {
    unsafe { cdio_paranoia_set_range(p, start, end) }
}

#[no_mangle]
pub unsafe extern "C" fn paranoia_cachemodel_size(p: *mut CdromParanoiaT, sectors: c_int) -> c_int {
    unsafe { cdio_paranoia_cachemodel_size(p, sectors) }
}

// CDDA compatibility aliases
#[no_mangle]
pub unsafe extern "C" fn cdda_find_a_cdrom(
    messagedest: c_int,
    ppsz_message: *mut *mut c_char,
) -> *mut CdromDriveT {
    unsafe { cdio_cddap_find_a_cdrom(messagedest, ppsz_message) }
}

#[no_mangle]
pub unsafe extern "C" fn cdda_identify(
    psz_device: *const c_char,
    messagedest: c_int,
    ppsz_message: *mut *mut c_char,
) -> *mut CdromDriveT {
    unsafe { cdio_cddap_identify(psz_device, messagedest, ppsz_message) }
}

#[no_mangle]
pub unsafe extern "C" fn cdda_open(d: *mut CdromDriveT) -> c_int {
    unsafe { cdio_cddap_open(d) }
}

#[no_mangle]
pub unsafe extern "C" fn cdda_close(d: *mut CdromDriveT) -> c_int {
    unsafe { cdio_cddap_close(d) }
}

#[no_mangle]
pub unsafe extern "C" fn cdda_read(
    d: *mut CdromDriveT,
    p_buffer: *mut c_void,
    beginsector: Lsn,
    sectors: c_long,
) -> c_long {
    unsafe { cdio_cddap_read(d, p_buffer, beginsector, sectors) }
}

#[no_mangle]
pub unsafe extern "C" fn cdda_track_firstsector(d: *mut CdromDriveT, i_track: u8) -> Lsn {
    unsafe { cdio_cddap_track_firstsector(d, i_track) }
}

#[no_mangle]
pub unsafe extern "C" fn cdda_track_lastsector(d: *mut CdromDriveT, i_track: u8) -> Lsn {
    unsafe { cdio_cddap_track_lastsector(d, i_track) }
}

#[no_mangle]
pub unsafe extern "C" fn cdda_tracks(d: *mut CdromDriveT) -> c_int {
    unsafe { cdio_cddap_tracks(d) }
}

#[no_mangle]
pub unsafe extern "C" fn cdda_disc_firstsector(d: *mut CdromDriveT) -> Lsn {
    unsafe { cdio_cddap_disc_firstsector(d) }
}

#[no_mangle]
pub unsafe extern "C" fn cdda_disc_lastsector(d: *mut CdromDriveT) -> Lsn {
    unsafe { cdio_cddap_disc_lastsector(d) }
}
