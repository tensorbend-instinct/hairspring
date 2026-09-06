// Vendored from xai-org/grok-build (Apache-2.0): the hashline_edit engine.
// The grok-build Tool-trait wrapper (mod.rs there) is NOT vendored - it is
// bound to their runtime; hairspring plugins wrap apply_edits directly.
pub mod apply;
pub mod range_policy;
pub mod types;
