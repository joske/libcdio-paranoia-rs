//! Backoff mechanism for handling persistent read errors.
//!
//! When CD drives encounter persistent errors in a specific area, backing off
//! (seeking away and back) can help by:
//! - Clearing the drive's internal cache
//! - Allowing the laser to recalibrate
//! - Giving the disc a chance to spin to a different position
//!
//! This module provides configurable backoff strategies for error recovery.

#![allow(dead_code)]

use std::time::{Duration, Instant};

/// Default number of errors before triggering backoff.
pub const DEFAULT_ERROR_THRESHOLD: u32 = 3;

/// Default backoff distance in sectors.
pub const DEFAULT_BACKOFF_SECTORS: i32 = 16;

/// Maximum backoff distance in sectors.
pub const MAX_BACKOFF_SECTORS: i32 = 128;

/// Default delay after backoff seek (milliseconds).
pub const DEFAULT_BACKOFF_DELAY_MS: u64 = 50;

/// Maximum delay after backoff (milliseconds).
pub const MAX_BACKOFF_DELAY_MS: u64 = 500;

/// Backoff strategy for handling persistent errors.
#[derive(Debug, Clone)]
pub struct BackoffStrategy {
    /// Number of consecutive errors at current position
    error_count: u32,
    /// Number of errors before triggering backoff
    error_threshold: u32,
    /// Current backoff distance in sectors
    backoff_sectors: i32,
    /// Base backoff distance
    base_backoff_sectors: i32,
    /// Current delay after backoff
    backoff_delay: Duration,
    /// Base delay after backoff
    base_backoff_delay: Duration,
    /// Number of backoffs performed at current position
    backoff_count: u32,
    /// Last position where error occurred
    last_error_sector: Option<i32>,
    /// Time of last backoff
    last_backoff_time: Option<Instant>,
    /// Total errors encountered
    total_errors: u64,
    /// Total backoffs performed
    total_backoffs: u64,
}

impl Default for BackoffStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl BackoffStrategy {
    /// Create a new backoff strategy with default settings.
    pub fn new() -> Self {
        Self {
            error_count: 0,
            error_threshold: DEFAULT_ERROR_THRESHOLD,
            backoff_sectors: DEFAULT_BACKOFF_SECTORS,
            base_backoff_sectors: DEFAULT_BACKOFF_SECTORS,
            backoff_delay: Duration::from_millis(DEFAULT_BACKOFF_DELAY_MS),
            base_backoff_delay: Duration::from_millis(DEFAULT_BACKOFF_DELAY_MS),
            backoff_count: 0,
            last_error_sector: None,
            last_backoff_time: None,
            total_errors: 0,
            total_backoffs: 0,
        }
    }

    /// Create a backoff strategy with custom settings.
    pub fn with_settings(error_threshold: u32, backoff_sectors: i32, backoff_delay_ms: u64) -> Self {
        Self {
            error_count: 0,
            error_threshold,
            backoff_sectors,
            base_backoff_sectors: backoff_sectors,
            backoff_delay: Duration::from_millis(backoff_delay_ms),
            base_backoff_delay: Duration::from_millis(backoff_delay_ms),
            backoff_count: 0,
            last_error_sector: None,
            last_backoff_time: None,
            total_errors: 0,
            total_backoffs: 0,
        }
    }

    /// Record an error at the given sector.
    /// Returns true if backoff should be triggered.
    pub fn record_error(&mut self, sector: i32) -> bool {
        self.total_errors += 1;

        // Check if this is a new error location
        if self.last_error_sector != Some(sector) {
            // New location - reset counters
            self.error_count = 1;
            self.backoff_count = 0;
            self.backoff_sectors = self.base_backoff_sectors;
            self.backoff_delay = self.base_backoff_delay;
            self.last_error_sector = Some(sector);
            return false;
        }

        // Same location - increment counter
        self.error_count += 1;

        // Check if we should trigger backoff
        self.error_count >= self.error_threshold
    }

    /// Record a successful read, resetting error state.
    pub fn record_success(&mut self, sector: i32) {
        // If we successfully read past an error location, reset
        if let Some(error_sector) = self.last_error_sector {
            if sector > error_sector {
                self.reset_error_state();
            }
        }
    }

    /// Reset the error state (after successful recovery or moving past error).
    pub fn reset_error_state(&mut self) {
        self.error_count = 0;
        self.backoff_count = 0;
        self.backoff_sectors = self.base_backoff_sectors;
        self.backoff_delay = self.base_backoff_delay;
        self.last_error_sector = None;
    }

    /// Get the backoff parameters for the current error state.
    /// Returns (seek_offset, delay) where seek_offset is negative (seek backward).
    pub fn get_backoff_params(&self) -> (i32, Duration) {
        (-self.backoff_sectors, self.backoff_delay)
    }

    /// Record that a backoff was performed, escalating parameters for next time.
    pub fn record_backoff(&mut self) {
        self.total_backoffs += 1;
        self.backoff_count += 1;
        self.error_count = 0; // Reset error count after backoff
        self.last_backoff_time = Some(Instant::now());

        // Exponential backoff - double distance and delay each time
        self.backoff_sectors = (self.backoff_sectors * 2).min(MAX_BACKOFF_SECTORS);
        self.backoff_delay = Duration::from_millis(
            (self.backoff_delay.as_millis() as u64 * 2).min(MAX_BACKOFF_DELAY_MS),
        );
    }

    /// Check if we should give up (too many backoffs at same location).
    pub fn should_give_up(&self, max_backoffs: u32) -> bool {
        self.backoff_count >= max_backoffs
    }

    /// Get the current error count.
    pub fn error_count(&self) -> u32 {
        self.error_count
    }

    /// Get the number of backoffs at current location.
    pub fn backoff_count(&self) -> u32 {
        self.backoff_count
    }

    /// Get total errors encountered.
    pub fn total_errors(&self) -> u64 {
        self.total_errors
    }

    /// Get total backoffs performed.
    pub fn total_backoffs(&self) -> u64 {
        self.total_backoffs
    }

    /// Get the last error sector.
    pub fn last_error_sector(&self) -> Option<i32> {
        self.last_error_sector
    }

    /// Get time since last backoff.
    pub fn time_since_backoff(&self) -> Option<Duration> {
        self.last_backoff_time.map(|t| t.elapsed())
    }

    /// Set the error threshold.
    pub fn set_error_threshold(&mut self, threshold: u32) {
        self.error_threshold = threshold.max(1);
    }

    /// Set the base backoff distance.
    pub fn set_backoff_sectors(&mut self, sectors: i32) {
        self.base_backoff_sectors = sectors.max(1);
        self.backoff_sectors = self.base_backoff_sectors;
    }

    /// Set the base backoff delay.
    pub fn set_backoff_delay(&mut self, delay_ms: u64) {
        self.base_backoff_delay = Duration::from_millis(delay_ms);
        self.backoff_delay = self.base_backoff_delay;
    }
}

/// Result of checking backoff state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackoffAction {
    /// Continue normally, no backoff needed.
    Continue,
    /// Perform backoff: seek backward by given sectors, wait given duration.
    Backoff { sectors: i32, delay: Duration },
    /// Give up: too many backoffs, skip this sector.
    GiveUp,
}

/// Helper to determine backoff action based on current state.
pub fn determine_backoff_action(
    strategy: &BackoffStrategy,
    needs_backoff: bool,
    max_backoffs: u32,
) -> BackoffAction {
    if !needs_backoff {
        return BackoffAction::Continue;
    }

    if strategy.should_give_up(max_backoffs) {
        return BackoffAction::GiveUp;
    }

    let (sectors, delay) = strategy.get_backoff_params();
    BackoffAction::Backoff {
        sectors: sectors.abs(), // Return positive value, caller decides direction
        delay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_creation() {
        let strategy = BackoffStrategy::new();
        assert_eq!(strategy.error_count(), 0);
        assert_eq!(strategy.backoff_count(), 0);
        assert_eq!(strategy.total_errors(), 0);
    }

    #[test]
    fn test_error_threshold() {
        let mut strategy = BackoffStrategy::new();

        // First two errors shouldn't trigger backoff
        assert!(!strategy.record_error(100));
        assert!(!strategy.record_error(100));

        // Third error should trigger backoff (default threshold is 3)
        assert!(strategy.record_error(100));

        assert_eq!(strategy.error_count(), 3);
        assert_eq!(strategy.total_errors(), 3);
    }

    #[test]
    fn test_different_sector_resets() {
        let mut strategy = BackoffStrategy::new();

        // Errors at sector 100
        strategy.record_error(100);
        strategy.record_error(100);

        // Error at different sector resets count
        assert!(!strategy.record_error(200));
        assert_eq!(strategy.error_count(), 1);
    }

    #[test]
    fn test_backoff_escalation() {
        let mut strategy = BackoffStrategy::new();

        let (initial_sectors, initial_delay) = strategy.get_backoff_params();

        strategy.record_backoff();

        let (new_sectors, new_delay) = strategy.get_backoff_params();

        // Should have doubled
        assert_eq!(new_sectors.abs(), initial_sectors.abs() * 2);
        assert_eq!(new_delay, initial_delay * 2);
    }

    #[test]
    fn test_backoff_max_limits() {
        let mut strategy = BackoffStrategy::new();

        // Perform many backoffs
        for _ in 0..10 {
            strategy.record_backoff();
        }

        let (sectors, delay) = strategy.get_backoff_params();

        // Should be capped at maximums
        assert!(sectors.abs() <= MAX_BACKOFF_SECTORS);
        assert!(delay.as_millis() <= MAX_BACKOFF_DELAY_MS as u128);
    }

    #[test]
    fn test_should_give_up() {
        let mut strategy = BackoffStrategy::new();

        // Not giving up initially
        assert!(!strategy.should_give_up(3));

        // Perform backoffs
        strategy.record_backoff();
        strategy.record_backoff();
        strategy.record_backoff();

        // Now should give up
        assert!(strategy.should_give_up(3));
    }

    #[test]
    fn test_success_resets() {
        let mut strategy = BackoffStrategy::new();

        // Build up errors
        strategy.record_error(100);
        strategy.record_error(100);
        strategy.record_backoff();

        // Success past error location resets
        strategy.record_success(101);

        assert_eq!(strategy.error_count(), 0);
        assert_eq!(strategy.backoff_count(), 0);
        assert_eq!(strategy.last_error_sector(), None);
    }

    #[test]
    fn test_determine_backoff_action() {
        let mut strategy = BackoffStrategy::new();

        // No backoff needed
        assert_eq!(
            determine_backoff_action(&strategy, false, 3),
            BackoffAction::Continue
        );

        // Backoff needed but not giving up
        assert!(matches!(
            determine_backoff_action(&strategy, true, 3),
            BackoffAction::Backoff { .. }
        ));

        // After too many backoffs, give up
        strategy.record_backoff();
        strategy.record_backoff();
        strategy.record_backoff();

        assert_eq!(
            determine_backoff_action(&strategy, true, 3),
            BackoffAction::GiveUp
        );
    }
}
