//! module cancel - Shared cancellation error helpers.
//!
//! Long-running async workers use a typed error instead of string matching so
//! callers can reliably distinguish user-requested cancellation from a real
//! failure.

use anyhow::Error;
use thiserror::Error;

// ---------------------------------------------- Types ----------------------------------------- //

/// Typed error returned when a batch worker is cancelled.
#[derive(Debug, Error)]
#[error("cancelled")]
pub struct Cancelled;

// ----------------------------------------- Public API ----------------------------------------- //

/// Returns true when an `anyhow::Error` represents a cooperative cancellation.
pub fn is_cancelled(error: &Error) -> bool {
    error.downcast_ref::<Cancelled>().is_some()
}
