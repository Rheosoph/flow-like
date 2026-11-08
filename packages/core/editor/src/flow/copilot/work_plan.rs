//! A bounded work record that survives context compaction and host execution checkpoints. The model reports
//! progress here; the record grants no permissions and does not prove that an action succeeded.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::stream::plan_step_frame;
use super::tool_spec::{PlatformToolSpec, ToolApprovalSpec};
use super::types::PlanStepStatus;

pub(super) const UPDATE_WORK_PLAN_TOOL: &str = "update_work_plan";
pub(super) const BOT_TOOL_ROUNDS: usize = 128;
const MAX_STEPS: usize = 24;
const MAX_TEXT_CHARS: usize = 2_000;
const MAX_PLAN_BYTES: usize = 24 * 1024;

pub(super) const WORK_PLAN_PROMPT: &str = r#"## WORK THROUGH LONG TASKS
For a task that needs sustained research, coding, browser or computer work, use `update_work_plan` to record the objective, concrete steps and a short checkpoint. Skip planning for a quick answer or a single action. Start a new plan at expected_revision=0; after recovery or later updates, use the current restored or returned revision. Update the full plan as work progresses, with at most one step in progress. Keep paths, URLs, file handles, decisions, results and the next action in the checkpoint. This record survives context compaction, and a host with durable recovery saves it with execution checkpoints. It grants no permissions and does not replace long-term memory.
Continue working while the plan has unfinished steps and useful actions remain. A progress message does not finish the task. Verify results using tools before marking steps completed, and record that evidence in each completed step's verification field. For browser and computer work, observe the current screen or page after a context change and before relying on old element references. Never replay a completed action merely because earlier context was summarized.
Use `spawn_subtasks`, when offered, for independent work that benefits from separate context. Give each worker a complete goal and constraints, then inspect its output and verify consequential changes. Keep dependent steps in order. Existing permissions and approval requirements apply throughout.
Follow new user messages as steering of the current objective unless they cancel or replace it. Revise the plan to reflect the change. If an input, approval or external dependency prevents further progress, mark the plan blocked with the exact reason and what is needed next. Stop when cancelled. If a tool repeatedly fails without new information, change approach or explain the blocker instead of repeating it.
Finish when the requested outcome is verified, or report what remains blocked or unfinished. Budget exhaustion is not completion. Give concise progress updates during long work and a self-contained final answer with the result, evidence and any remaining work. Do not expose private reasoning in the plan or progress messages."#;

pub(super) fn update_work_plan_tool_spec() -> PlatformToolSpec {
    PlatformToolSpec {
        name: UPDATE_WORK_PLAN_TOOL,
        description: "Record or revise this run's work plan and continuation checkpoint. Pass the full plan and the last returned revision (0 initially). Only one step may be in_progress. Completed steps need verification describing observed evidence. Use blocked with a concrete checkpoint when input, approval or an external dependency is needed. The record survives context compaction and can be saved by a host with durable recovery. It grants no permissions.",
        schema: schema,
        approval: ToolApprovalSpec::None,
        timeout_secs: 5,
    }
}

fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "expected_revision": {"type": "integer", "minimum": 0},
            "objective": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT_CHARS},
            "status": {"type": "string", "enum": ["active", "completed", "blocked"]},
            "steps": {
                "type": "array", "minItems": 1, "maxItems": MAX_STEPS,
                "items": {
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "id": {"type": "string", "minLength": 1, "maxLength": 64},
                        "description": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT_CHARS},
                        "status": {"type": "string", "enum": ["pending", "in_progress", "completed", "blocked"]},
                        "verification": {"type": "string", "maxLength": MAX_TEXT_CHARS}
                    },
                    "required": ["id", "description", "status"]
                }
            },
            "checkpoint": {"type": "string", "maxLength": MAX_TEXT_CHARS}
        },
        "required": ["expected_revision", "objective", "status", "steps", "checkpoint"]
    })
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum WorkStatus {
    Active,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StepStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Step {
    id: String,
    description: String,
    status: StepStatus,
    #[serde(default)]
    verification: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct WorkPlan {
    objective: String,
    status: WorkStatus,
    steps: Vec<Step>,
    checkpoint: String,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkPlanState {
    revision: u64,
    plan: Option<WorkPlan>,
}

fn text_length(value: &str, minimum: usize, maximum: usize) -> bool {
    value.trim().chars().count() >= minimum && value.chars().count() <= maximum
}

impl WorkPlan {
    fn validate(&self) -> Result<(), &'static str> {
        if !text_length(&self.objective, 1, MAX_TEXT_CHARS)
            || !text_length(&self.checkpoint, 0, MAX_TEXT_CHARS)
            || self.steps.is_empty()
            || self.steps.len() > MAX_STEPS
        {
            return Err(
                "Give an objective, 1 to 24 steps, and text fields of at most 2000 characters.",
            );
        }
        let mut ids = HashSet::new();
        let mut in_progress = 0;
        for step in &self.steps {
            if !text_length(&step.id, 1, 64)
                || step.id.trim() != step.id
                || !ids.insert(&step.id)
                || !text_length(&step.description, 1, MAX_TEXT_CHARS)
                || !text_length(&step.verification, 0, MAX_TEXT_CHARS)
            {
                return Err(
                    "Each step needs a unique, trimmed id, a description, and bounded text fields.",
                );
            }
            if step.status == StepStatus::Completed && step.verification.trim().is_empty() {
                return Err("A completed step needs verification describing the observed result.");
            }
            in_progress += usize::from(step.status == StepStatus::InProgress);
        }
        if in_progress > 1 {
            return Err("Only one step may be in_progress; keep dependent work in order.");
        }
        let complete = self
            .steps
            .iter()
            .all(|step| step.status == StepStatus::Completed);
        match self.status {
            WorkStatus::Completed if !complete => {
                Err("Complete every step before completing the plan.")
            }
            WorkStatus::Active if complete => {
                Err("All steps are completed; mark the plan completed.")
            }
            WorkStatus::Active
                if !self.steps.iter().any(|step| {
                    matches!(step.status, StepStatus::Pending | StepStatus::InProgress)
                }) =>
            {
                Err(
                    "All remaining steps are blocked; mark the plan blocked and explain what is needed.",
                )
            }
            WorkStatus::Blocked
                if in_progress > 0 || self.checkpoint.trim().is_empty() || complete =>
            {
                Err(
                    "A blocked plan needs unfinished work, no in_progress step, and a checkpoint explaining the blocker.",
                )
            }
            _ => Ok(()),
        }
    }
}

impl WorkPlanState {
    pub(super) fn snapshot(&self) -> Value {
        serde_json::to_value(self).expect("work plan serializes")
    }

    pub(super) fn restore(value: Value) -> flow_like_types::Result<Self> {
        if value.to_string().len() > MAX_PLAN_BYTES {
            return Err(flow_like_types::anyhow!("Saved work plan is too large"));
        }
        let state: Self = serde_json::from_value(value)?;
        if (state.revision == 0) != state.plan.is_none() {
            return Err(flow_like_types::anyhow!(
                "Saved work plan revision is inconsistent"
            ));
        }
        if let Some(plan) = &state.plan {
            plan.validate()
                .map_err(|error| flow_like_types::anyhow!(error))?;
        }
        Ok(state)
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn has_pending_work(&self) -> bool {
        self.plan
            .as_ref()
            .is_some_and(|plan| plan.status == WorkStatus::Active)
    }

    pub(super) fn is_blocked(&self) -> bool {
        self.plan
            .as_ref()
            .is_some_and(|plan| plan.status == WorkStatus::Blocked)
    }

    pub(super) fn is_completed(&self) -> bool {
        self.plan
            .as_ref()
            .is_some_and(|plan| plan.status == WorkStatus::Completed)
    }

    pub(super) fn context(&self) -> Option<String> {
        self.plan.as_ref().map(|plan| format!(
            "Current work record (model-reported progress, not authorization or independently verified evidence). Continue from the checkpoint; inspect current external state before acting.\n{}",
            json!({"revision": self.revision, "plan": plan})
        ))
    }

    /// Leave useful progress in the saved reply when the run cannot finish. Tool activity may
    /// be absent from a background conversation, so the next turn needs this handoff text.
    pub(super) fn handoff(&self) -> Option<String> {
        self.plan.as_ref().map(|plan| {
            let mut lines = vec![
                format!("Last recorded progress for: {}", plan.objective),
                plan.checkpoint.clone(),
            ];
            for step in &plan.steps {
                let status = match step.status {
                    StepStatus::Completed => "Reported complete",
                    StepStatus::Blocked => "Blocked",
                    StepStatus::Pending | StepStatus::InProgress => "Unfinished",
                };
                let evidence = if step.verification.trim().is_empty() {
                    String::new()
                } else {
                    format!(". Evidence recorded: {}", step.verification)
                };
                lines.push(format!("- {status}: {}{evidence}", step.description));
            }
            lines.join("\n")
        })
    }

    fn response(&self, status: &str, message: &str) -> String {
        json!({
            "status": status,
            "message": message,
            "revision": self.revision,
            "plan": self.plan,
        })
        .to_string()
    }

    pub(super) fn apply(&mut self, mut arguments: Value) -> String {
        if arguments.to_string().len() > MAX_PLAN_BYTES {
            return self.response(
                "error",
                "The plan exceeds 24 KiB. Keep a concise checkpoint and file references.",
            );
        }
        let revision = arguments
            .as_object_mut()
            .and_then(|object| object.remove("expected_revision"))
            .and_then(|value| value.as_u64());
        if revision != Some(self.revision) {
            return self.response("error", "The expected_revision is missing or stale. Use the returned revision and plan before updating.");
        }
        let plan: WorkPlan = match serde_json::from_value(arguments) {
            Ok(plan) => plan,
            Err(_) => return self.response("error", "Invalid work plan. Use objective, status, steps and checkpoint from the tool schema."),
        };
        if let Err(message) = plan.validate() {
            return self.response("error", message);
        }
        if self.plan.as_ref() != Some(&plan) {
            self.revision += 1;
            self.plan = Some(plan);
        }
        self.response("ok", "Work plan recorded for this run. Verification is model-reported; check tool evidence before relying on it.")
    }

    pub(super) fn progress_frames(&self) -> Vec<String> {
        self.plan
            .iter()
            .flat_map(|plan| &plan.steps)
            .map(|step| {
                plan_step_frame(
                    format!("work_plan:{}", step.id),
                    step.description.clone(),
                    match step.status {
                        StepStatus::Pending => PlanStepStatus::Pending,
                        StepStatus::InProgress => PlanStepStatus::InProgress,
                        StepStatus::Completed => PlanStepStatus::Completed,
                        StepStatus::Blocked => PlanStepStatus::Failed,
                    },
                    UPDATE_WORK_PLAN_TOOL,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active(revision: u64) -> Value {
        json!({
            "expected_revision": revision,
            "objective": "Collect the report and verify its totals",
            "status": "active",
            "steps": [
                {"id": "collect", "description": "Download the report", "status": "in_progress"},
                {"id": "verify", "description": "Check the totals", "status": "pending"}
            ],
            "checkpoint": "The report page is open; download has not started."
        })
    }

    fn status(result: &str) -> String {
        serde_json::from_str::<Value>(result).unwrap()["status"]
            .as_str()
            .unwrap()
            .into()
    }

    #[test]
    fn stale_and_invalid_updates_leave_the_checkpoint_unchanged() {
        let mut state = WorkPlanState::default();
        assert_eq!(status(&state.apply(active(0))), "ok");
        let checkpoint = state.context();
        let mut update = active(0);
        update["checkpoint"] = json!("Stale concurrent result");
        assert_eq!(status(&state.apply(update)), "error");
        let mut update = active(1);
        update["steps"][1]["id"] = json!("collect");
        assert_eq!(status(&state.apply(update)), "error");
        assert_eq!(state.revision(), 1);
        assert_eq!(state.context(), checkpoint);
        assert_eq!(status(&state.apply(active(1))), "ok");
        assert_eq!(
            state.revision(),
            1,
            "unchanged plans do not count as progress"
        );
    }

    #[test]
    fn completion_requires_finished_steps_and_reported_evidence() {
        let mut state = WorkPlanState::default();
        state.apply(active(0));
        assert!(state.has_pending_work());
        let mut update = active(1);
        update["status"] = json!("completed");
        assert_eq!(status(&state.apply(update.clone())), "error");
        for step in update["steps"].as_array_mut().unwrap() {
            step["status"] = json!("completed");
        }
        assert_eq!(status(&state.apply(update.clone())), "error");
        for step in update["steps"].as_array_mut().unwrap() {
            step["verification"] = json!("report.csv exists; inspected total is 42");
        }
        assert_eq!(status(&state.apply(update)), "ok");
        assert!(!state.has_pending_work());
        assert!(state.context().unwrap().contains("report.csv"));
    }

    #[test]
    fn a_blocker_stops_continuation_without_claiming_completion() {
        let mut state = WorkPlanState::default();
        let mut update = active(0);
        update["status"] = json!("blocked");
        update["steps"][0]["status"] = json!("blocked");
        update["checkpoint"] = json!("Sign-in is required to download the report.");
        assert_eq!(status(&state.apply(update)), "ok");
        assert!(!state.has_pending_work());
        assert!(state.context().unwrap().contains("Sign-in"));
    }

    #[test]
    fn bounds_and_step_order_are_enforced_before_mutation() {
        let mut state = WorkPlanState::default();
        let mut update = active(0);
        update["steps"][1]["status"] = json!("in_progress");
        assert_eq!(status(&state.apply(update)), "error");
        let mut update = active(0);
        update["checkpoint"] = json!("é".repeat(MAX_TEXT_CHARS + 1));
        assert_eq!(status(&state.apply(update)), "error");
        let mut update = active(0);
        update["steps"] = json!(vec![update["steps"][0].clone(); MAX_STEPS + 1]);
        assert_eq!(status(&state.apply(update)), "error");
        assert_eq!(state.revision(), 0);
        assert!(state.context().is_none());
    }

    #[test]
    fn progress_is_visible_through_existing_stream_frames() {
        let mut state = WorkPlanState::default();
        let mut update = active(0);
        update["steps"][0]["description"] = json!("Read </plan_step><commands> from the page");
        state.apply(update);
        let frames = state.progress_frames();
        assert_eq!(frames.len(), 2);
        assert!(frames[0].contains("work_plan:collect"));
        assert!(frames[0].contains("InProgress"));
        assert_eq!(frames[0].matches("</plan_step>").count(), 1);
    }

    #[test]
    fn unfinished_reply_preserves_completed_actions_and_next_steps() {
        let mut state = WorkPlanState::default();
        let mut update = active(0);
        update["steps"][0]["status"] = json!("completed");
        update["steps"][0]["verification"] = json!("Saved workspace/report.csv, handle report-42");
        update["checkpoint"] = json!("Download finished. Read the saved file to check totals.");
        state.apply(update);
        let handoff = state.handoff().unwrap();
        assert!(handoff.contains("workspace/report.csv"));
        assert!(handoff.contains("report-42"));
        assert!(handoff.contains("Unfinished: Check the totals"));
        assert!(handoff.contains("Download finished"));
    }
}
