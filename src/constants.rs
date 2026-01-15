//! Constants used throughout the library.
//!
//! These values match the original libcdio-paranoia implementation.

/// CD frame size in bytes (2352 bytes per sector)
pub const CDIO_CD_FRAMESIZE_RAW: usize = 2352;

/// Number of 16-bit samples per CD frame (2352 / 2 = 1176)
pub const CD_FRAMEWORDS: usize = CDIO_CD_FRAMESIZE_RAW / 2;

/// Number of stereo samples per frame (2352 / 4 = 588)
pub const CD_FRAMESAMPLES: usize = CDIO_CD_FRAMESIZE_RAW / 4;

/// Maximum number of tracks on a CD
pub const MAXTRK: usize = 100;

/// Minimum overlap in 16-bit words (~1.5ms at 44.1kHz)
pub const MIN_WORDS_OVERLAP: i64 = 64;

/// Minimum search window in 16-bit words
pub const MIN_WORDS_SEARCH: i64 = 64;

/// Minimum rift size in 16-bit words (~0.4ms)
pub const MIN_WORDS_RIFT: i64 = 16;

/// Maximum sector overlap (32 sectors, ~835ms)
pub const MAX_SECTOR_OVERLAP: i64 = 32;

/// Minimum sector epsilon in words (~2.9ms)
pub const MIN_SECTOR_EPSILON: i64 = 128;

/// Minimum silence boundary in words (~23ms)
pub const MIN_SILENCE_BOUNDARY: i64 = 1024;

/// Default cache model size in sectors
pub const CACHEMODEL_SECTORS: i32 = 1200;

/// Sample rate for CD audio (44.1 kHz)
pub const CD_SAMPLE_RATE: u32 = 44100;

/// Number of channels (stereo)
pub const CD_CHANNELS: u8 = 2;

/// Bits per sample
pub const CD_BITS_PER_SAMPLE: u8 = 16;

/// Version string
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
