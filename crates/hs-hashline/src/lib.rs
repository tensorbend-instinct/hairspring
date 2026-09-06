//! Hashline anchor engine (LINE:HASH), vendored (Apache-2.0) from
//! xai-org/grok-build for the edit-path bake-off: anchors generated on read,
//! validated against the pre-edit snapshot, bounded shifted-anchor recovery.
//! No hand-rolled parts.
pub mod anchor;
pub mod config;
pub mod edit;
pub mod render;
pub mod hash;
pub mod mutate;
pub mod scheme;
