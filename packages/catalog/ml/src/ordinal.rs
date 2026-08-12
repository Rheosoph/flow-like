//! Ordinal regression nodes.
//!
//! For targets whose classes carry a meaningful order (`1 < 2 < ... < 6`, or
//! `low < medium < high`). A plain classifier discards that ordering and a plain regressor invents
//! distances between levels, so these get their own estimators — see the `flow-like-ordinal` crate.

pub mod adjacent_category;
pub mod continuation_ratio;
pub mod frank_hall;
pub mod logistic;
pub mod metrics;
pub mod neural;
pub mod ridge;
