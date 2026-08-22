use flow_like::flow::node::NodeScores;

/// Scores shared by pure, deterministic, local-only utility nodes.
pub fn pure_scores() -> NodeScores {
    NodeScores::new()
        .set_privacy(10)
        .set_security(10)
        .set_performance(10)
        .set_governance(10)
        .set_reliability(10)
        .set_cost(10)
        .build()
}

pub mod array;
pub mod bool;
pub mod bytes;
pub mod crypto;
pub mod csv;
pub mod cuid;
pub mod datetime;
pub mod encoding;
pub mod execution;
pub mod float;
pub mod format;
pub mod hash;
pub mod identifiers;
pub mod int;
pub mod json;
pub mod map;
pub mod math;
pub mod md;
pub mod random;
pub mod set;
pub mod string;
pub mod types;
pub mod user;
pub mod vector;
