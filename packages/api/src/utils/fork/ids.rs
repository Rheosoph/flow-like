//! Deterministic destination ids for forks.
//!
//! Every row, node, pin and layer a fork creates gets its id from
//! `(seed, source id)` instead of a random draw, so re-running any fork
//! step after a crash or a lost commit produces the very same ids and the
//! DB writes collapse into `ON CONFLICT DO NOTHING`.

use flow_like_types::create_id;

pub use flow_like::app::remap::derive_id;

/// A fresh, random seed for a fork that has no job row (offline bundles).
pub fn fresh_seed() -> String {
    create_id()
}
