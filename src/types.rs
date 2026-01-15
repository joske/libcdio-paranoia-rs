//! Core type definitions.

use bitflags::bitflags;

/// Logical Sector Number - absolute sector position on disc
pub type Lsn = i32;

/// Track number (1-99, or 0xAA for lead-out)
pub type TrackNum = u8;

bitflags! {
    /// Paranoia mode flags controlling error correction behavior.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(transparent)]
    pub struct ParanoiaMode: i32 {
        /// No fixups - disable all error correction
        const DISABLE = 0x00;
        /// Verify data integrity in overlap areas
        const VERIFY = 0x01;
        /// Fragment mode (unsupported)
        const FRAGMENT = 0x02;
        /// Perform overlapped reads
        const OVERLAP = 0x04;
        /// Scratch detection (unsupported)
        const SCRATCH = 0x08;
        /// Repair mode (unsupported)
        const REPAIR = 0x10;
        /// Do not skip failed reads (retry max_retries times)
        const NEVERSKIP = 0x20;
        /// Maximum paranoia - all supported modes enabled
        const FULL = 0xFF;
    }
}

impl Default for ParanoiaMode {
    fn default() -> Self {
        ParanoiaMode::FULL
    }
}

/// Callback event types reported during reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum ParanoiaCallback {
    /// Read operation
    Read = 0,
    /// Verifying jitter
    Verify = 1,
    /// Fixed edge jitter
    FixupEdge = 2,
    /// Fixed atom jitter
    FixupAtom = 3,
    /// Scratch detected (unsupported)
    Scratch = 4,
    /// Repair performed (unsupported)
    Repair = 5,
    /// Skip exhausted retry
    Skip = 6,
    /// Drift detected and corrected
    Drift = 7,
    /// Backoff (unsupported)
    Backoff = 8,
    /// Dynamic overlap adjust
    Overlap = 9,
    /// Fixed dropped bytes
    FixupDropped = 10,
    /// Fixed duplicate bytes
    FixupDuped = 11,
    /// Hard read error
    ReadError = 12,
    /// Bad cache management
    CacheError = 13,
    /// Wrote block
    Wrote = 14,
    /// Finished writing
    Finished = 15,
}

impl ParanoiaCallback {
    /// Convert to string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            ParanoiaCallback::Read => "read",
            ParanoiaCallback::Verify => "verify",
            ParanoiaCallback::FixupEdge => "fixup_edge",
            ParanoiaCallback::FixupAtom => "fixup_atom",
            ParanoiaCallback::Scratch => "scratch",
            ParanoiaCallback::Repair => "repair",
            ParanoiaCallback::Skip => "skip",
            ParanoiaCallback::Drift => "drift",
            ParanoiaCallback::Backoff => "backoff",
            ParanoiaCallback::Overlap => "overlap",
            ParanoiaCallback::FixupDropped => "fixup_dropped",
            ParanoiaCallback::FixupDuped => "fixup_duped",
            ParanoiaCallback::ReadError => "read_error",
            ParanoiaCallback::CacheError => "cache_error",
            ParanoiaCallback::Wrote => "wrote",
            ParanoiaCallback::Finished => "finished",
        }
    }
}

impl From<i32> for ParanoiaCallback {
    fn from(value: i32) -> Self {
        match value {
            0 => ParanoiaCallback::Read,
            1 => ParanoiaCallback::Verify,
            2 => ParanoiaCallback::FixupEdge,
            3 => ParanoiaCallback::FixupAtom,
            4 => ParanoiaCallback::Scratch,
            5 => ParanoiaCallback::Repair,
            6 => ParanoiaCallback::Skip,
            7 => ParanoiaCallback::Drift,
            8 => ParanoiaCallback::Backoff,
            9 => ParanoiaCallback::Overlap,
            10 => ParanoiaCallback::FixupDropped,
            11 => ParanoiaCallback::FixupDuped,
            12 => ParanoiaCallback::ReadError,
            13 => ParanoiaCallback::CacheError,
            14 => ParanoiaCallback::Wrote,
            15 => ParanoiaCallback::Finished,
            _ => ParanoiaCallback::Read,
        }
    }
}

/// Message destination options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum MessageDest {
    /// Discard messages
    ForgetIt = 0,
    /// Print messages to stderr
    PrintIt = 1,
    /// Log messages to buffer
    LogIt = 2,
}

/// Table of Contents entry
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct TocEntry {
    /// Track number
    pub track: u8,
    /// Start sector (LSN)
    pub start_sector: Lsn,
}

/// Per-sample flags used in verification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct SampleFlags(pub u8);

impl SampleFlags {
    /// No flags set
    pub const NONE: SampleFlags = SampleFlags(0);
    /// Sample is at an edge
    pub const EDGE: SampleFlags = SampleFlags(0x01);
    /// Sample has been blanked
    pub const BLANKED: SampleFlags = SampleFlags(0x02);
    /// Sample has been verified
    pub const VERIFIED: SampleFlags = SampleFlags(0x04);

    pub fn is_edge(self) -> bool {
        self.0 & Self::EDGE.0 != 0
    }

    pub fn is_blanked(self) -> bool {
        self.0 & Self::BLANKED.0 != 0
    }

    pub fn is_verified(self) -> bool {
        self.0 & Self::VERIFIED.0 != 0
    }
}

/// Jitter test flags for testing/simulation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum JitterTest {
    /// Small jitter simulation
    Small = 1,
    /// Large jitter simulation
    Large = 2,
    /// Massive jitter simulation
    Massive = 3,
}
