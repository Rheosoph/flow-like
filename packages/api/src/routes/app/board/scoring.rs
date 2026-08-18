use flow_like::flow::board::{
    Board, ExecutionMode, ExecutionStage,
    summary::{BoardEntryNode, BoardSummaryMetrics},
};
use flow_like::flow::execution::LogLevel;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait,
    FromJsonQueryResult, QueryFilter,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::entity::app_board_score;

/// Nodes scoring below this value in a category are considered "bad patterns".
/// Matches the red band threshold used by the frontend `ScoreBar`.
pub const SCORE_FLAG_THRESHOLD: u8 = 4;

/// Upper bound on the number of distinct flagged patterns persisted per board.
/// Flags are a diagnostic sample, not an exhaustive ledger: capping keeps the
/// stored JSON small and bounded even for very large boards (10k+ nodes).
pub const MAX_FLAGGED_PATTERNS_PER_BOARD: usize = 100;

/// Aggregated quality scores for a board (MIN across nodes, 0-10, higher = better).
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoardScores {
    pub security: u8,
    pub privacy: u8,
    pub performance: u8,
    pub governance: u8,
    pub reliability: u8,
    pub cost: u8,
}

/// A node type dragging a board's score down in a category. Deduplicated per
/// board: `score` is the lowest observed and `count` is how many node instances
/// of this type tripped the threshold in this category.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlaggedPattern {
    pub node: String,
    /// Display name of the node type as placed. Empty on rows persisted before it was recorded.
    #[serde(default)]
    pub friendly_name: String,
    pub category: String,
    pub score: u8,
    #[serde(default = "default_pattern_count")]
    pub count: u32,
}

fn default_pattern_count() -> u32 {
    1
}

/// Full result of aggregating a board's node scores.
#[derive(Clone, Debug)]
pub struct BoardScoreComputation {
    pub scores: Option<BoardScores>,
    pub worst_score: u8,
    pub node_count: u32,
    pub scored_node_count: u32,
    pub connection_count: u32,
    pub flagged_patterns: Vec<FlaggedPattern>,
}

/// Cached, lightweight board summary metadata persisted alongside the scores so
/// board summaries can be served without reading every board from object storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct BoardSummaryMeta {
    pub name: String,
    pub description: String,
    pub stage: ExecutionStage,
    pub execution_mode: ExecutionMode,
    pub log_level: LogLevel,
    pub version: (u32, u32, u32),
    pub connection_count: u32,
    pub variable_count: u32,
    pub layer_count: u32,
    pub comment_count: u32,
    /// Distinct node type names on the board, sorted. `None` on rows persisted before this
    /// field existed; the summaries fallback path backfills it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_types: Option<Vec<String>>,
    /// Nodes flagged as entry points (`start == true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_nodes: Option<Vec<BoardEntryNode>>,
    /// The board's `updated_at` when this summary was computed. Lets a client with a local copy
    /// decide whether to pull the board at all. `None` on rows persisted before this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<std::time::SystemTime>,
    /// Overview metrics (wasm usage, variable/layer breakdowns, total node count). `None` on
    /// rows persisted before this field; the summaries fallback path backfills it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<BoardSummaryMetrics>,
}

/// Build the cached summary metadata for a board. `connection_count` is taken
/// from [`compute_board_score`] so connections are not counted twice.
pub fn board_summary_meta(board: &Board, connection_count: u32) -> BoardSummaryMeta {
    let node_types = board.summary_node_types();
    let entry_nodes = board.summary_entry_nodes();
    BoardSummaryMeta {
        name: board.name.clone(),
        description: board.description.clone(),
        stage: board.stage.clone(),
        execution_mode: board.execution_mode.clone(),
        log_level: board.log_level,
        version: board.version,
        connection_count,
        variable_count: board.variables.len() as u32,
        layer_count: board.layers.len() as u32,
        comment_count: board.comments.len() as u32,
        node_types: Some(node_types),
        entry_nodes: Some(entry_nodes),
        updated_at: Some(board.updated_at),
        metrics: Some(board.summary_metrics()),
    }
}

/// Compute the aggregated scores, counts and flagged patterns for a board.
/// Scores aggregate as the MIN per category across all non-`reroute` nodes that
/// carry scores. Flagged patterns are nodes scoring below [`SCORE_FLAG_THRESHOLD`].
pub fn compute_board_score(board: &Board) -> BoardScoreComputation {
    let mut node_count = 0u32;
    let mut scored_node_count = 0u32;
    let mut connection_count = 0u32;
    let mut min_scores: Option<BoardScores> = None;
    // Deduplicate flags per (node type, category): keep the worst score seen and
    // count how many node instances tripped the threshold. This collapses the
    // node-instance multiplier so a board with thousands of identical bad nodes
    // stores a handful of entries instead of one per node.
    let mut flagged: HashMap<(String, &'static str), (u8, u32)> = HashMap::new();
    let mut friendly_names: HashMap<String, String> = HashMap::new();

    for node in board.nodes.values() {
        if node.name == "reroute" {
            continue;
        }
        node_count += 1;
        for pin in node.pins.values() {
            connection_count += pin.connected_to.len() as u32;
        }

        if let Some(ref s) = node.scores {
            scored_node_count += 1;
            min_scores = Some(match min_scores {
                None => BoardScores {
                    security: s.security,
                    privacy: s.privacy,
                    performance: s.performance,
                    governance: s.governance,
                    reliability: s.reliability,
                    cost: s.cost,
                },
                Some(prev) => BoardScores {
                    security: prev.security.min(s.security),
                    privacy: prev.privacy.min(s.privacy),
                    performance: prev.performance.min(s.performance),
                    governance: prev.governance.min(s.governance),
                    reliability: prev.reliability.min(s.reliability),
                    cost: prev.cost.min(s.cost),
                },
            });

            for (category, score) in [
                ("security", s.security),
                ("privacy", s.privacy),
                ("performance", s.performance),
                ("governance", s.governance),
                ("reliability", s.reliability),
                ("cost", s.cost),
            ] {
                if score < SCORE_FLAG_THRESHOLD {
                    let entry = flagged
                        .entry((node.name.clone(), category))
                        .or_insert((score, 0));
                    entry.0 = entry.0.min(score);
                    entry.1 += 1;
                    friendly_names
                        .entry(node.name.clone())
                        .or_insert_with(|| node.friendly_name.clone());
                }
            }
        }
    }
    connection_count /= 2;

    let mut flagged_patterns: Vec<FlaggedPattern> = flagged
        .into_iter()
        .map(|((node, category), (score, count))| FlaggedPattern {
            friendly_name: friendly_names.get(&node).cloned().unwrap_or_default(),
            node,
            category: category.to_string(),
            score,
            count,
        })
        .collect();

    // Worst first, then most frequent, then stable by name. Cap the persisted
    // set so a single row stays bounded regardless of board size.
    flagged_patterns.sort_by(|a, b| {
        a.score
            .cmp(&b.score)
            .then_with(|| b.count.cmp(&a.count))
            .then_with(|| a.node.cmp(&b.node))
            .then_with(|| a.category.cmp(&b.category))
    });
    flagged_patterns.truncate(MAX_FLAGGED_PATTERNS_PER_BOARD);

    let worst_score = match &min_scores {
        Some(s) => s
            .security
            .min(s.privacy)
            .min(s.performance)
            .min(s.governance)
            .min(s.reliability)
            .min(s.cost),
        None => 10,
    };

    BoardScoreComputation {
        scores: min_scores,
        worst_score,
        node_count,
        scored_node_count,
        connection_count,
        flagged_patterns,
    }
}

/// Persist a mutated floating board the way every graph-changing API path must: write the
/// object, refresh its summary row, and pin the in-memory board to the new ETag.
///
/// The summary row is what `/board/summaries` serves without touching storage, so it is only
/// as fresh as the last writer that refreshed it — every path that changes nodes, variables,
/// layers or comments has to go through here. The upsert overlaps the object put (both take
/// `&board`), so an edit pays for the slower of the two rather than their sum. A summary
/// failure is logged, never surfaced: the board write is the source of truth and the next
/// summaries request backfills a missing row.
///
/// Returns the store's [`PutResult`] once the object write succeeded; the caller decides
/// whether the board is still needed afterwards, so seeding is the caller's line.
pub async fn save_board_and_refresh_summary(
    state: &crate::state::AppState,
    app_id: &str,
    board: &Board,
) -> flow_like_types::Result<flow_like::flow_like_storage::object_store::PutResult> {
    let (put, summary) = flow_like_types::tokio::join!(
        board.save(None),
        persist_board_score(&state.db, app_id, board)
    );
    if let Err(err) = summary {
        tracing::warn!(
            "failed to refresh board summary for {app_id}/{}: {err:?}",
            board.id
        );
    }
    put
}

/// Upsert the persisted [`app_board_score`] row for a single board.
///
/// Runs inline (awaited) so it completes within the request — safe under
/// serverless runtimes (e.g. Lambda) where detached background tasks are frozen.
pub async fn persist_board_score<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    board: &Board,
) -> flow_like_types::Result<()> {
    let computation = compute_board_score(board);
    persist_board_score_with(db, app_id, board, &computation).await
}

/// Like [`persist_board_score`] but reuses an already-computed
/// [`BoardScoreComputation`] to avoid aggregating the board twice.
pub async fn persist_board_score_with<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    board: &Board,
    computation: &BoardScoreComputation,
) -> flow_like_types::Result<()> {
    let now = chrono::Utc::now().naive_utc();

    let scores = computation.scores.clone().unwrap_or(BoardScores {
        security: 10,
        privacy: 10,
        performance: 10,
        governance: 10,
        reliability: 10,
        cost: 10,
    });

    let flagged_json = serde_json::to_value(&computation.flagged_patterns).ok();
    let summary = Some(serde_json::to_value(board_summary_meta(
        board,
        computation.connection_count,
    ))?);

    let existing = app_board_score::Entity::find()
        .filter(app_board_score::Column::AppId.eq(app_id))
        .filter(app_board_score::Column::BoardId.eq(&board.id))
        .one(db)
        .await?;

    match existing {
        Some(model) => {
            let mut active: app_board_score::ActiveModel = model.into();
            active.security = Set(scores.security as i32);
            active.privacy = Set(scores.privacy as i32);
            active.performance = Set(scores.performance as i32);
            active.governance = Set(scores.governance as i32);
            active.reliability = Set(scores.reliability as i32);
            active.cost = Set(scores.cost as i32);
            active.worst_score = Set(computation.worst_score as i32);
            active.node_count = Set(computation.node_count as i32);
            active.scored_node_count = Set(computation.scored_node_count as i32);
            active.connection_count = Set(computation.connection_count as i32);
            active.flagged_patterns = Set(flagged_json);
            active.summary = Set(summary);
            active.computed_at = Set(now);
            active.updated_at = Set(now);
            active.update(db).await?;
        }
        None => {
            let active = app_board_score::ActiveModel {
                id: Set(flow_like_types::create_id()),
                app_id: Set(app_id.to_string()),
                board_id: Set(board.id.clone()),
                security: Set(scores.security as i32),
                privacy: Set(scores.privacy as i32),
                performance: Set(scores.performance as i32),
                governance: Set(scores.governance as i32),
                reliability: Set(scores.reliability as i32),
                cost: Set(scores.cost as i32),
                worst_score: Set(computation.worst_score as i32),
                node_count: Set(computation.node_count as i32),
                scored_node_count: Set(computation.scored_node_count as i32),
                connection_count: Set(computation.connection_count as i32),
                flagged_patterns: Set(flagged_json),
                summary: Set(summary),
                computed_at: Set(now),
                updated_at: Set(now),
            };
            active.insert(db).await?;
        }
    }

    Ok(())
}

/// Remove the persisted score row for a board (e.g. on board delete).
pub async fn delete_board_score<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    board_id: &str,
) -> flow_like_types::Result<()> {
    app_board_score::Entity::delete_many()
        .filter(app_board_score::Column::AppId.eq(app_id))
        .filter(app_board_score::Column::BoardId.eq(board_id))
        .exec(db)
        .await?;
    Ok(())
}
