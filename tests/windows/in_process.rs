//! The Windows tests that build the crate in-process with `cfg(test)`, so they can reach `App`
//! and the test seams, which stay absent from production. This also means
//! `show_json_issue`/`show_json_valid` compile to their `#[cfg(test)]` thread-local-recording
//! variants here, so a real (blocking, modal) MessageBoxW is never shown by this target.
//!
//! They share one target so the crate (and its unit tests) is compiled once for all of them,
//! instead of once per file. Run one file's tests with its module path as the filter, e.g.
//! `cargo test --test in_process markdown_live::`.
#![cfg(windows)]

include!("../../src/lib.rs");

mod support;

#[path = "editing.rs"]
mod editing;
#[path = "image_preview.rs"]
mod image_preview;
#[path = "json_commands.rs"]
mod json_commands;
#[path = "markdown_live.rs"]
mod markdown_live;
#[path = "markdown_preview.rs"]
mod markdown_preview;
#[path = "open_file.rs"]
mod open_file;
#[path = "save_file.rs"]
mod save_file;
