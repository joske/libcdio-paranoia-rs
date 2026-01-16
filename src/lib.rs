//! # cdio-paranoia
//!
//! A Rust implementation of libcdio-paranoia - CD audio extraction with error correction.
//!
//! This library provides both an idiomatic Rust API and a C-compatible ABI for
//! drop-in replacement of the original libcdio-paranoia.
//!
//! ## Features
//!
//! - Two-stage verification with overlap detection and rift repair
//! - Automatic jitter correction
//! - Detection and repair of dropped/duplicated samples
//! - Dynamic overlap adjustment based on drive characteristics
//! - Progress callbacks for monitoring extraction status
//!
//! ## Example (Rust API)
//!
//! ```no_run
//! use cdio_paranoia::{CdromDrive, Paranoia, ParanoiaMode};
//!
//! // Open a CD drive
//! let drive = CdromDrive::open(Some("/dev/cdrom")).expect("Failed to open drive");
//!
//! // Initialize paranoia
//! let mut paranoia = Paranoia::new(drive);
//! paranoia.set_mode(ParanoiaMode::FULL);
//!
//! // Read sectors with error correction
//! for sector in paranoia.iter() {
//!     // Process audio data...
//! }
//! ```

#![allow(
    clippy::missing_safety_doc,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_lossless
)]

pub mod constants;
pub mod error;
pub mod types;

mod backoff;
mod block;
mod cdda;
mod consensus;
mod gap;
mod isort;
mod overlap;
mod paranoia;

pub mod ffi;

pub use block::{CBlock, VFragment};
pub use cdda::CdromDrive;
pub use constants::*;
pub use error::{Error, Result};
pub use paranoia::Paranoia;
pub use types::*;
