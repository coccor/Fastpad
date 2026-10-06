//! Live Markdown (live mode spec): rendered Markdown inside the Scintilla editor.

pub mod blocks;
pub mod reveal;
pub mod spans;
pub mod styler;
pub mod styles;

/// Live is unavailable for documents larger than this (live mode spec §4).
pub const LIVE_MAX_BYTES: usize = 1_048_576;
