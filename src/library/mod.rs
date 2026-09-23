//! The note library: a folder of plain text files seen as notes, plus sparse organizational
//! metadata (notebooks, tags, favorites, pins) kept in `.fastpad\library.ini` and attached to
//! files by path, file ID and content fingerprint. Nothing here touches a window.

pub mod ids;
pub mod local;
pub mod model;
pub mod ops;
pub mod store;
