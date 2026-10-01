//! Workflow run summaries and terminal report emission.

use super::stream_events::send_correlated_stream_json_event;
use super::workflow_state::{
    EXTERNAL_EXTENSION_CONTINUATION_GRANT, MAX_EXTERNAL_EARNED_WALL_CLOCK,
    MAX_EXTERNAL_WORKFLOW_CONTINUATIONS, MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS,
    NESTED_RUN_WALL_CLOCK_BUDGET, WorkflowToolLoopSnapshot, WorkflowToolLoopState,
    scoped_commit_budget, scoped_edit_budget, scoped_operation_budget,
};
use flow_like::flow::copilot::{BoardScopePlan, WorkflowSessionSnapshot};
use flow_like_types::tokio_util::sync::CancellationToken;
use std::{
    sync::{Arc, Mutex as StdMutex},
    time::Instant,
};
use tauri::ipc::Channel;

const RUN_SUMMARY_EVENT_KIND: &str = "run_summary";

fn workflow_run_summary_budget_entry(used: u64, limit: u64) -> serde_json::Value {
    serde_json::json!({ "used": used, "limit": limit })
}

/// Build the single structured per-run summary payload from state the workflow loop already
/// tracks. The frame rides the existing `tool_end` stream tag (without a `tool_call_id`, which the
/// process-step views ignore) so every provider path reuses one pipe, and the debug report pins it
/// at maximum retention.
/// Frontend-facing projection of the accepted plan: what the build was split into, what reached the
/// board, and what is still outstanding. This is what lets a caller report an honest partial instead
/// of presenting a half-built board as a finished one.
pub(super) fn workflow_run_summary_scope_plan(plan: &BoardScopePlan) -> serde_json::Value {
    serde_json::json!({
        "strategy": plan.strategy,
        "segment_count": plan.segment_count(),
        "segments_applied": plan.committed_count(),
        "segments_remaining": plan.segment_count().saturating_sub(plan.committed_count()),
        "remaining_titles": plan.uncommitted_titles(),
        "segments": plan
            .segments
            .iter()
            .map(|segment| serde_json::json!({
                "id": segment.id,
                "title": segment.title,
                "board_ref": segment.board_ref,
                "applied": plan.committed.iter().any(|id| id == &segment.id),
            }))
            .collect::<Vec<_>>(),
        "revisions": plan.revisions,
        "rationale": plan.rationale,
    })
}

/// Collect the unimplemented stubs a build handed back to the user.
///
/// The specialist is told never to abandon a build over one impossible unit; it emits a
/// correctly-typed function whose body logs `NOT IMPLEMENTED: <what is missing>` instead. Scanning
/// the committed source for that marker is what turns those holes into something the orchestrator
/// can actually tell the user about — otherwise the workflow looks finished and silently is not.
///
/// Deliberately a text scan over the retained source rather than a graph walk: the marker lives in a
/// literal the model wrote, and the source is the one representation available at summary time on
/// every provider path.
pub(super) fn collect_unimplemented_stubs(source: &str) -> Vec<serde_json::Value> {
    const MAX_STUBS_REPORTED: usize = 16;
    let mut stubs: Vec<serde_json::Value> = Vec::new();
    let mut current_function: Option<String> = None;
    let mut depth: i32 = 0;

    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("function ")
            && depth == 0
        {
            let name = rest
                .split(['(', '<', ' '])
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            if !name.is_empty() {
                current_function = Some(name);
            }
        }

        if let Some(index) = line.find(flow_like::copilot::prompts::UNIMPLEMENTED_STUB_MARKER)
            && stubs.len() < MAX_STUBS_REPORTED
        {
            // The marker lives inside a double-quoted message literal, so the detail ends at that
            // literal's closing quote — not at the end of the line, which still carries the rest of
            // the call (`", toast: true })`).
            let detail = line
                [index + flow_like::copilot::prompts::UNIMPLEMENTED_STUB_MARKER.len()..]
                .split('"')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            stubs.push(serde_json::json!({
                "function": current_function.clone(),
                "detail": detail,
            }));
        }

        depth += line.matches('{').count() as i32;
        depth -= line.matches('}').count() as i32;
        if depth <= 0 {
            depth = 0;
            current_function = None;
        }
    }

    stubs
}

pub(super) fn workflow_run_summary_payload(
    outcome: &str,
    provider: &str,
    model: &str,
    duration_ms: u64,
    phases: u32,
    continuations_used: u32,
    continuations_limit: u32,
    snapshot: Option<&WorkflowToolLoopSnapshot>,
    applied_commands: usize,
) -> serde_json::Value {
    let mut diagnostics_by_code = std::collections::BTreeMap::<String, u64>::new();
    for entry in snapshot
        .map(|snapshot| snapshot.last_structured_diagnostics.as_slice())
        .unwrap_or_default()
    {
        let Some(code) = entry.get("code").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let occurrences = entry
            .get("occurrences")
            .and_then(serde_json::Value::as_u64)
            .filter(|count| *count > 0)
            .unwrap_or(1);
        let total = diagnostics_by_code.entry(code.to_string()).or_default();
        *total = total.saturating_add(occurrences);
    }
    let retained_draft = snapshot
        .and_then(|snapshot| {
            if snapshot.flowscript_draft_retained {
                snapshot
                    .flowscript_draft_id
                    .as_deref()
                    .map(|id| (id, snapshot.flowscript_revision))
            } else if snapshot.typed_draft_retained {
                snapshot
                    .typed_draft_id
                    .as_deref()
                    .map(|id| (id, snapshot.typed_revision))
            } else {
                None
            }
        })
        .map(|(id, revision)| serde_json::json!({ "id": id, "revision": revision }))
        .unwrap_or(serde_json::Value::Null);
    let scope_plan = snapshot.and_then(|snapshot| snapshot.scope_plan.as_ref());
    let manual_steps = snapshot
        .and_then(|snapshot| {
            snapshot
                .retained_full_source
                .as_deref()
                .or(snapshot.last_flowscript.as_deref())
        })
        .map(collect_unimplemented_stubs)
        .unwrap_or_default();
    serde_json::json!({
        "kind": RUN_SUMMARY_EVENT_KIND,
        "tool": RUN_SUMMARY_EVENT_KIND,
        "status": if matches!(outcome, "provider_failure" | "incomplete") { "error" } else { "done" },
        "outcome": outcome,
        "provider": provider,
        "model": model,
        "duration_ms": duration_ms,
        "phases": phases,
        "scope_plan": scope_plan
            .map(workflow_run_summary_scope_plan)
            .unwrap_or(serde_json::Value::Null),
        // Units the specialist could not build and replaced with a typed stub. Reported so the
        // orchestrator can hand them to the user as work only they can finish; an unreported stub
        // is a workflow the user believes is complete.
        "manual_steps": manual_steps,
        // How much wall clock this run EARNED by proving progress, so a long build is auditable
        // after the fact rather than looking like an unexplained multi-hour hang.
        "time_budget": {
            "limit_secs": snapshot
                .and_then(|snapshot| snapshot.nested_wall_clock_budget)
                .unwrap_or(NESTED_RUN_WALL_CLOCK_BUDGET)
                .as_secs(),
            "granted_extensions": snapshot.map_or(0, |snapshot| snapshot.granted_time_extensions),
            "earned_secs": snapshot.map_or(0, |snapshot| snapshot.earned_wall_clock.as_secs()),
            "ceiling_secs": MAX_EXTERNAL_EARNED_WALL_CLOCK.as_secs(),
            "last_rationale": snapshot
                .and_then(|snapshot| snapshot.last_extension_rationale.clone())
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        },
        "budget": {
            "checks": workflow_run_summary_budget_entry(
                snapshot.map_or(0, |snapshot| u64::from(snapshot.edit_attempts)),
                u64::from(snapshot
                    .and_then(|snapshot| snapshot.edit_attempt_budget)
                    .unwrap_or_else(|| scoped_edit_budget(scope_plan))),
            ),
            "source_ops": workflow_run_summary_budget_entry(
                snapshot.map_or(0, |snapshot| u64::from(snapshot.flowscript_operation_attempts)),
                u64::from(snapshot
                    .and_then(|snapshot| snapshot.flowscript_operation_budget)
                    .unwrap_or_else(|| scoped_operation_budget(scope_plan))),
            ),
            "commits": workflow_run_summary_budget_entry(
                snapshot.map_or(0, |snapshot| u64::from(snapshot.flowscript_commit_attempts)),
                u64::from(snapshot
                    .and_then(|snapshot| snapshot.commit_attempt_budget)
                    .unwrap_or_else(|| scoped_commit_budget(scope_plan))),
            ),
            "stalled": workflow_run_summary_budget_entry(
                snapshot.map_or(0, |snapshot| u64::from(snapshot.stalled_edit_attempts)),
                u64::from(MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS),
            ),
            "continuations": workflow_run_summary_budget_entry(
                u64::from(continuations_used),
                // Earned time raises the continuation cap, so report the budget this run actually
                // ran under rather than the flat starting value.
                u64::from(continuations_limit).max(u64::from(
                    snapshot.map_or(MAX_EXTERNAL_WORKFLOW_CONTINUATIONS, |snapshot| {
                        snapshot.continuation_budget.unwrap_or_else(|| MAX_EXTERNAL_WORKFLOW_CONTINUATIONS.saturating_add(
                            snapshot
                                .granted_time_extensions
                                .saturating_mul(EXTERNAL_EXTENSION_CONTINUATION_GRANT),
                        ))
                    }),
                )),
            ),
        },
        "diagnostics_by_code": diagnostics_by_code,
        "retained_draft": retained_draft,
        "review_notes": snapshot.map_or(0, |snapshot| snapshot.last_review_notes),
        "applied_commands": applied_commands,
        "shared_session": snapshot
            .and_then(|snapshot| snapshot.shared_session.as_ref())
            .map(serde_json::to_value)
            .transpose()
            .ok()
            .flatten()
            .unwrap_or(serde_json::Value::Null),
    })
}

/// Emits exactly one `run_summary` frame when a FlowPilot run reaches any terminal path. The
/// emission is Drop-based so early error returns, cancellations, and provider failures cannot
/// skip it; success paths set the resolved outcome and applied-command count before the emitter
/// goes out of scope.
pub(super) struct WorkflowRunSummaryEmitter {
    pub(super) channel: Channel<String>,
    pub(super) parent_request_id: Option<String>,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) started: Instant,
    pub(super) cancellation: CancellationToken,
    pub(super) workflow_state: Option<Arc<StdMutex<WorkflowToolLoopState>>>,
    /// Bits/core publishes the same provider-neutral session lifecycle directly rather than
    /// mirroring it through the external adapter's WorkflowToolLoopState.
    pub(super) shared_session_snapshot: Option<Arc<StdMutex<Option<WorkflowSessionSnapshot>>>>,
    pub(super) phases: u32,
    pub(super) continuations_used: u32,
    pub(super) continuations_limit: u32,
    pub(super) budget_incomplete: bool,
    pub(super) outcome: Option<&'static str>,
    pub(super) applied_commands: usize,
}

impl WorkflowRunSummaryEmitter {
    pub(super) fn new(
        channel: Channel<String>,
        parent_request_id: Option<String>,
        provider: &str,
        model: &str,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            channel,
            parent_request_id,
            provider: provider.to_string(),
            model: model.to_string(),
            started: Instant::now(),
            cancellation,
            workflow_state: None,
            shared_session_snapshot: None,
            phases: 0,
            continuations_used: 0,
            continuations_limit: u32::from(MAX_EXTERNAL_WORKFLOW_CONTINUATIONS),
            budget_incomplete: false,
            outcome: None,
            applied_commands: 0,
        }
    }

    pub(super) fn attach_workflow_state(
        &mut self,
        state: Option<Arc<StdMutex<WorkflowToolLoopState>>>,
    ) {
        self.workflow_state = state;
    }

    pub(super) fn attach_shared_session_snapshot(
        &mut self,
        snapshot: Arc<StdMutex<Option<WorkflowSessionSnapshot>>>,
    ) {
        self.shared_session_snapshot = Some(snapshot);
    }

    pub(super) fn set_continuation_limit(&mut self, limit: u32) {
        self.continuations_limit = limit;
    }

    pub(super) fn record_phase(&mut self) {
        self.phases = self.phases.saturating_add(1);
    }

    pub(super) fn record_continuation(&mut self) {
        self.continuations_used = self.continuations_used.saturating_add(1);
    }

    pub(super) fn mark_budget_incomplete(&mut self) {
        self.budget_incomplete = true;
    }

    pub(super) fn set_applied_commands(&mut self, applied_commands: usize) {
        self.applied_commands = applied_commands;
    }

    pub(super) fn set_outcome(&mut self, outcome: &'static str) {
        self.outcome = Some(outcome);
    }

    pub(super) fn snapshot(&self) -> Option<WorkflowToolLoopSnapshot> {
        let adapter_snapshot = self
            .workflow_state
            .as_ref()
            .and_then(|state| state.lock().ok().map(|state| state.snapshot()));
        let shared_session = self
            .shared_session_snapshot
            .as_ref()
            .and_then(|sink| sink.lock().ok().and_then(|snapshot| snapshot.clone()));
        if adapter_snapshot.is_none() && shared_session.is_none() {
            return None;
        }
        let mut snapshot = adapter_snapshot.unwrap_or_default();
        if shared_session.is_some() {
            snapshot.shared_session = shared_session;
        }
        Some(snapshot)
    }

    /// Classify the terminal outcome from state the run already tracks. Queued work outranks a
    /// trailing provider error (the mutation was handed off); an edit request that ends cleanly
    /// without queueing anything is honestly incomplete, not completed.
    pub(super) fn resolve_outcome(&mut self, run_error: bool, workflow_edit_request: bool) {
        let queued = self.snapshot().is_some_and(|snapshot| snapshot.queued);
        self.outcome = Some(if self.cancellation.is_cancelled() {
            "cancelled"
        } else if queued {
            "committed"
        } else if self.budget_incomplete {
            "incomplete"
        } else if run_error {
            "provider_failure"
        } else if workflow_edit_request
            && (self.workflow_state.is_some() || self.shared_session_snapshot.is_some())
        {
            "incomplete"
        } else {
            "completed"
        });
    }
}

impl Drop for WorkflowRunSummaryEmitter {
    fn drop(&mut self) {
        let snapshot = self.snapshot();
        let outcome = self.outcome.unwrap_or_else(|| {
            // Unset outcome means the run left through an early error return (or panic unwind).
            if self.cancellation.is_cancelled() {
                "cancelled"
            } else if snapshot.as_ref().is_some_and(|snapshot| snapshot.queued) {
                "committed"
            } else if self.budget_incomplete {
                "incomplete"
            } else {
                "provider_failure"
            }
        });
        let payload = workflow_run_summary_payload(
            outcome,
            &self.provider,
            &self.model,
            u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            self.phases.max(1),
            self.continuations_used,
            self.continuations_limit,
            snapshot.as_ref(),
            self.applied_commands,
        );
        send_correlated_stream_json_event(
            &self.channel,
            "tool_end",
            &payload,
            self.parent_request_id.as_deref(),
        );
    }
}
