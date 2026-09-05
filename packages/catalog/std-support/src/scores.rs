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
