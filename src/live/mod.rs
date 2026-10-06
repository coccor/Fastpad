//! Live Markdown (live mode spec): rendered Markdown inside the Scintilla editor.

pub mod spans;

/// Live is unavailable for documents larger than this (live mode spec §4).
pub const LIVE_MAX_BYTES: usize = 1_048_576;
