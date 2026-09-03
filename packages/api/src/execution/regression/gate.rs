//! The publish/promote gate: fold the newest completed suite run of the
//! event's own suite for a board version into a verdict. One suite-row read
//! plus one indexed query (`[appId, boardId, boardVersion, createdAt]`,
//! narrowed by `suiteId`), no bucket reads.

use flow_like::flow::regression::GateMode;
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::{
    entity::{regression_suite, regression_suite_run},
    error::ApiError,
    state::AppState,
};

use super::{SUITE_RUN_COMPLETED, parse_gate_mode};

/// The gate signal for one `(event, board, version)` candidate. `Fail` means
/// the newest completed suite run recorded at least one `REGRESSED` case —
/// verdict-vs-baseline, so authored `test*` failures count too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateVerdict {
    Pass {
        suite_run_id: String,
    },
    Fail {
        suite_run_id: String,
        regressed: i32,
    },
    /// The event has no suite, or no completed run of it exists for this
    /// exact board version (draft targets and floating variants land here too
    /// — the gate keys on pinned versions only).
    NotRun,
}

/// The event's suite projection row, by the `[appId, eventId]` unique.
async fn suite_for_event(
    state: &AppState,
    app_id: &str,
    event_id: &str,
) -> Result<Option<regression_suite::Model>, ApiError> {
    regression_suite::Entity::find()
        .filter(regression_suite::Column::AppId.eq(app_id))
        .filter(regression_suite::Column::EventId.eq(event_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to load the suite row: {e}")))
}

/// The newest completed run of the event's own suite for `(board, version)`
/// decides the verdict; `errored` runs are no verdict and are skipped by the
/// status filter. Scoped by `suiteId` — two events sharing one board must
/// never read each other's verdicts — so an event without a suite is
/// `NotRun`.
pub async fn gate_verdict(
    state: &AppState,
    app_id: &str,
    event_id: &str,
    board_id: &str,
    board_version: (u32, u32, u32),
) -> Result<GateVerdict, ApiError> {
    let Some(suite) = suite_for_event(state, app_id, event_id).await? else {
        return Ok(GateVerdict::NotRun);
    };
    suite_gate_verdict(state, app_id, &suite.id, board_id, board_version).await
}

async fn suite_gate_verdict(
    state: &AppState,
    app_id: &str,
    suite_id: &str,
    board_id: &str,
    board_version: (u32, u32, u32),
) -> Result<GateVerdict, ApiError> {
    let label = crate::routes::app::events::dotted_version_key(board_version);
    let newest = regression_suite_run::Entity::find()
        .filter(regression_suite_run::Column::AppId.eq(app_id))
        .filter(regression_suite_run::Column::BoardId.eq(board_id))
        .filter(regression_suite_run::Column::SuiteId.eq(suite_id))
        .filter(regression_suite_run::Column::BoardVersion.eq(label))
        .filter(regression_suite_run::Column::Status.eq(SUITE_RUN_COMPLETED))
        .order_by_desc(regression_suite_run::Column::CreatedAt)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Gate verdict query failed: {e}")))?;
    Ok(match newest {
        None => GateVerdict::NotRun,
        Some(run) if run.regressed > 0 => GateVerdict::Fail {
            suite_run_id: run.id,
            regressed: run.regressed,
        },
        Some(run) => GateVerdict::Pass {
            suite_run_id: run.id,
        },
    })
}

/// The gate as `promote_canary` consumes it: the event's suite gate mode plus
/// the verdict for the promoted target. `None` when the event has no suite or
/// its gate is `Off`.
#[derive(Clone, Debug)]
pub struct PromotionGate {
    pub mode: GateMode,
    pub verdict: GateVerdict,
}

impl PromotionGate {
    /// Whether this gate refuses the promotion: `Block` mode and a `Fail`
    /// verdict. `NotRun` never blocks — the gate keys on pinned, suite-run
    /// versions and refusing everything else would wedge floating targets.
    pub fn blocks(&self) -> bool {
        self.mode == GateMode::Block && matches!(self.verdict, GateVerdict::Fail { .. })
    }
}

/// Resolve the gate for promoting `board_id`@`board_version` on an event. One
/// projection-row read (by the `[appId, eventId]` unique) plus, when armed,
/// the one indexed suite-scoped verdict query. A floating target
/// (`board_version: None`) is `NotRun`.
pub async fn promotion_gate(
    state: &AppState,
    app_id: &str,
    event_id: &str,
    board_id: &str,
    board_version: Option<(u32, u32, u32)>,
) -> Result<Option<PromotionGate>, ApiError> {
    let Some(suite) = suite_for_event(state, app_id, event_id).await? else {
        return Ok(None);
    };
    let mode = parse_gate_mode(&suite.gate_mode).unwrap_or_default();
    if mode == GateMode::Off {
        return Ok(None);
    }
    let verdict = match board_version {
        Some(version) => suite_gate_verdict(state, app_id, &suite.id, board_id, version).await?,
        None => GateVerdict::NotRun,
    };
    Ok(Some(PromotionGate { mode, verdict }))
}
