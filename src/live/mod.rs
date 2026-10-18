//! The Live Markdown view (2026-10-18 Live editor design spec): a FastPad-drawn view of a Markdown
//! tab's Scintilla document. Everything here except the layout and the window is pure.

pub mod lines;
pub mod offsets;

/// Larger documents stay in the source view (spec §3).
pub const LIVE_MAX_BYTES: usize = 4 * 1024 * 1024;
