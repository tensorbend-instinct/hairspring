//! Codex-format `apply_patch` engine, vendored (Apache-2.0): grammar parser,
//! fuzzy sequence matcher, pure patch application. No hand-rolled parts.
pub mod apply;
pub mod errors;
pub mod parser;
pub mod seek_sequence;
