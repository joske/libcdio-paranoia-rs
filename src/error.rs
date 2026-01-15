//! Error types for the library.

use thiserror::Error;

/// Result type alias using our Error type
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during CD audio extraction
#[derive(Error, Debug)]
pub enum Error {
    /// Unable to set CDROM to read audio mode
    #[error("Unable to set CDROM to read audio mode")]
    CddaModeError,

    /// Unable to read table of contents
    #[error("Unable to read table of contents: {0}")]
    TocReadError(String),

    /// CDROM reporting illegal number of tracks
    #[error("CDROM reporting illegal number of tracks")]
    IllegalTrackCount,

    /// Could not read any data from drive
    #[error("Could not read any data from drive")]
    NoDataRead,

    /// Unknown, unrecoverable error reading data
    #[error("Unknown, unrecoverable error reading data")]
    UnknownReadError,

    /// Unable to identify CDROM model
    #[error("Unable to identify CDROM model")]
    IdentifyError,

    /// CDROM reporting illegal table of contents
    #[error("CDROM reporting illegal table of contents")]
    IllegalToc,

    /// Unaddressable sector
    #[error("Unaddressable sector: {0}")]
    UnaddressableSector(i32),

    /// Interface not supported
    #[error("Interface not supported")]
    InterfaceNotSupported,

    /// Drive is neither a CDROM nor a WORM device
    #[error("Drive is neither a CDROM nor a WORM device")]
    NotCdromDevice,

    /// Permission denied on cdrom device
    #[error("Permission denied on cdrom device: {0}")]
    PermissionDenied(String),

    /// Device not open
    #[error("Device not open")]
    DeviceNotOpen,

    /// Invalid track number
    #[error("Invalid track number: {0}")]
    InvalidTrack(u8),

    /// Track is not audio data
    #[error("Track {0} is not audio data")]
    NotAudioTrack(u8),

    /// No audio tracks on disc
    #[error("No audio tracks on disc")]
    NoAudioTracks,

    /// No medium present
    #[error("No medium present")]
    NoMedium,

    /// Option not supported by drive
    #[error("Option not supported by drive")]
    OptionNotSupported,

    /// Transport error
    #[error("Transport error: {0}")]
    TransportError(TransportError),

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Sector skip (after max retries exhausted)
    #[error("Sector skip at LSN {0} after {1} retries")]
    SectorSkip(i32, i32),
}

/// Transport-level errors
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum TransportError {
    #[error("Success")]
    Ok = 0,
    #[error("Error writing packet command to device")]
    WriteError = 1,
    #[error("Error reading command from device")]
    ReadError = 2,
    #[error("SCSI packet data underrun (too little data)")]
    Underrun = 3,
    #[error("SCSI packet data overrun (too much data)")]
    Overrun = 4,
    #[error("Illegal SCSI request (rejected by target)")]
    Illegal = 5,
    #[error("Medium error reading data")]
    Medium = 6,
    #[error("Device busy")]
    Busy = 7,
    #[error("Device not ready")]
    NotReady = 8,
    #[error("Target hardware fault")]
    Fault = 9,
    #[error("Unspecified error")]
    Unknown = 10,
    #[error("Drive lost streaming")]
    Streaming = 11,
}

impl From<i32> for TransportError {
    fn from(value: i32) -> Self {
        match value {
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
            11 => TransportError::Streaming,
            _ => TransportError::Unknown,
        }
    }
}
