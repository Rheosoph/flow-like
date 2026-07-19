//! Deterministic quality metrics for FlowPilot workflow generation.
//!
//! The evaluator intentionally has no model, catalog, parser, or board dependencies. Production
//! code and offline benchmark runners can record the outcome of each candidate with the same
//! schema, then aggregate runs without replaying them. Diagnostic keys should normally be the
//! stable structured diagnostic id; callers that do not have one yet can use the stable code.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

pub const FLOWPILOT_GENERATION_EVALUATION_VERSION: &str = "flowpilot.generation-evaluation/v1";

fn evaluation_version() -> String {
    FLOWPILOT_GENERATION_EVALUATION_VERSION.to_string()
}

/// One generated candidate and the result of each validation boundary.
///
/// `elapsed_ms` is measured from the start of the containing run, not the duration of just this
/// attempt. `accepted` means that the candidate was atomically committed/applied; it only counts
/// as a success when all three validation flags are also true.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowPilotGenerationAttemptRecord {
    /// One-based attempt number. Aggregation sorts attempts by this value.
    pub attempt_index: u32,
    pub elapsed_ms: u64,
    pub parse_valid: bool,
    pub typed_valid: bool,
    pub reconcile_valid: bool,
    pub accepted: bool,
    /// Stable structured diagnostic ids, falling back to stable diagnostic codes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostic_keys: Vec<String>,
}

impl FlowPilotGenerationAttemptRecord {
    /// A typed candidate is only valid when the preceding parse boundary also passed.
    pub fn is_typed_valid(&self) -> bool {
        self.parse_valid && self.typed_valid
    }

    /// The first useful definition of a valid workflow candidate: it parsed, typechecked, and
    /// reconciled against the live catalog. Atomic application is measured separately as success.
    pub fn is_valid(&self) -> bool {
        self.is_typed_valid() && self.reconcile_valid
    }

    pub fn is_success(&self) -> bool {
        self.is_valid() && self.accepted
    }

    fn validation_depth(&self) -> u8 {
        if !self.parse_valid {
            0
        } else if !self.typed_valid {
            1
        } else if !self.reconcile_valid {
            2
        } else {
            3
        }
    }

    fn unique_diagnostics(&self) -> BTreeSet<&str> {
        self.diagnostic_keys
            .iter()
            .map(String::as_str)
            .filter(|key| !key.is_empty())
            .collect()
    }
}

/// Result of the explicit capability-planning gate.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowPilotPlanOutcome {
    #[default]
    NotAssessed,
    Feasible,
    Infeasible,
}

/// Terminal state matters for board-after-run metrics: a still-running report is deliberately
/// excluded instead of being counted as an empty failure.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowPilotEvaluationRunStatus {
    #[default]
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl FlowPilotEvaluationRunStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// All deterministic evidence needed to score one workflow-generation run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlowPilotGenerationRunRecord {
    #[serde(default = "evaluation_version")]
    pub version: String,
    pub run_id: String,
    #[serde(default)]
    pub status: FlowPilotEvaluationRunStatus,
    #[serde(default)]
    pub plan_outcome: FlowPilotPlanOutcome,
    /// Absent when the run is still active or the board could not be inspected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_board_node_count: Option<usize>,
    #[serde(default)]
    pub attempts: Vec<FlowPilotGenerationAttemptRecord>,
}

/// A rate plus its raw counts. Keeping the denominator in the public schema prevents dashboards
/// from presenting `0 / 0` as a real zero-percent result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowPilotRateMetric {
    pub numerator: u64,
    pub denominator: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
}

impl FlowPilotRateMetric {
    fn new(numerator: u64, denominator: u64) -> Self {
        Self {
            numerator,
            denominator,
            rate: (denominator > 0).then(|| numerator as f64 / denominator as f64),
        }
    }
}

/// Distribution of elapsed time from run start to the first parse+type+reconcile-valid candidate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowPilotDurationMetric {
    pub samples: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_ms: Option<u64>,
}

impl FlowPilotDurationMetric {
    fn from_values(mut values: Vec<u64>) -> Self {
        if values.is_empty() {
            return Self {
                samples: 0,
                mean_ms: None,
                median_ms: None,
                min_ms: None,
                max_ms: None,
            };
        }

        values.sort_unstable();
        let samples = values.len() as u64;
        let total = values.iter().map(|value| *value as u128).sum::<u128>();
        let middle = values.len() / 2;
        let median_ms = if values.len() % 2 == 0 {
            (values[middle - 1] as f64 + values[middle] as f64) / 2.0
        } else {
            values[middle] as f64
        };

        Self {
            samples,
            mean_ms: Some(total as f64 / samples as f64),
            median_ms: Some(median_ms),
            min_ms: values.first().copied(),
            max_ms: values.last().copied(),
        }
    }
}

/// Aggregate generation quality. All at-1 and within-N rates use runs with at least one candidate
/// as their denominator. Repair rates use adjacent attempts whose preceding candidate had at
/// least one diagnostic. Planning and final-board rates expose their own eligible denominators.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowPilotGenerationScorecard {
    #[serde(default = "evaluation_version")]
    pub version: String,
    pub runs_total: u64,
    pub runs_with_attempts: u64,
    pub attempts_total: u64,
    pub parse_at_1: FlowPilotRateMetric,
    pub typed_valid_at_1: FlowPilotRateMetric,
    pub reconcile_valid_at_1: FlowPilotRateMetric,
    pub success_within_1_attempt: FlowPilotRateMetric,
    pub success_within_3_attempts: FlowPilotRateMetric,
    /// A repair regresses when it falls to an earlier validation phase, or remains in the same
    /// phase while increasing the number of distinct diagnostics.
    pub diagnostic_regression: FlowPilotRateMetric,
    /// A repair repeats at least one stable diagnostic from the preceding attempt.
    pub repeated_diagnostic: FlowPilotRateMetric,
    /// An attempt at position three or later returns to a complete diagnostic set seen before the
    /// immediately preceding attempt (`A -> B -> A`).
    pub diagnostic_oscillation: FlowPilotRateMetric,
    pub time_to_first_valid: FlowPilotDurationMetric,
    pub infeasible_plan: FlowPilotRateMetric,
    pub empty_board_after_run: FlowPilotRateMetric,
}

impl FlowPilotGenerationScorecard {
    pub fn from_runs(runs: &[FlowPilotGenerationRunRecord]) -> Self {
        let mut runs_with_attempts = 0_u64;
        let mut attempts_total = 0_u64;
        let mut parse_at_1 = 0_u64;
        let mut typed_valid_at_1 = 0_u64;
        let mut reconcile_valid_at_1 = 0_u64;
        let mut success_within_1 = 0_u64;
        let mut success_within_3 = 0_u64;
        let mut repair_transitions = 0_u64;
        let mut diagnostic_regressions = 0_u64;
        let mut repeated_diagnostics = 0_u64;
        let mut oscillation_opportunities = 0_u64;
        let mut diagnostic_oscillations = 0_u64;
        let mut first_valid_times = Vec::new();
        let mut assessed_plans = 0_u64;
        let mut infeasible_plans = 0_u64;
        let mut inspected_terminal_boards = 0_u64;
        let mut empty_terminal_boards = 0_u64;

        for run in runs {
            attempts_total += run.attempts.len() as u64;

            match run.plan_outcome {
                FlowPilotPlanOutcome::NotAssessed => {}
                FlowPilotPlanOutcome::Feasible => assessed_plans += 1,
                FlowPilotPlanOutcome::Infeasible => {
                    assessed_plans += 1;
                    infeasible_plans += 1;
                }
            }

            if run.status.is_terminal()
                && let Some(node_count) = run.final_board_node_count
            {
                inspected_terminal_boards += 1;
                empty_terminal_boards += u64::from(node_count == 0);
            }

            let mut attempts = run.attempts.iter().collect::<Vec<_>>();
            attempts.sort_by_key(|attempt| attempt.attempt_index);
            let Some(first) = attempts.first() else {
                continue;
            };

            runs_with_attempts += 1;
            parse_at_1 += u64::from(first.parse_valid);
            typed_valid_at_1 += u64::from(first.is_typed_valid());
            reconcile_valid_at_1 += u64::from(first.is_valid());
            success_within_1 += u64::from(first.is_success());
            success_within_3 +=
                u64::from(attempts.iter().take(3).any(|attempt| attempt.is_success()));

            if let Some(attempt) = attempts.iter().find(|attempt| attempt.is_valid()) {
                first_valid_times.push(attempt.elapsed_ms);
            }

            let diagnostic_sets = attempts
                .iter()
                .map(|attempt| attempt.unique_diagnostics())
                .collect::<Vec<_>>();

            for index in 1..attempts.len() {
                let previous = attempts[index - 1];
                let current = attempts[index];
                let previous_diagnostics = &diagnostic_sets[index - 1];
                let current_diagnostics = &diagnostic_sets[index];

                if !previous_diagnostics.is_empty() {
                    repair_transitions += 1;
                    let regressed = current.validation_depth() < previous.validation_depth()
                        || (current.validation_depth() == previous.validation_depth()
                            && current_diagnostics.len() > previous_diagnostics.len());
                    diagnostic_regressions += u64::from(regressed);
                    repeated_diagnostics += u64::from(
                        previous_diagnostics
                            .intersection(current_diagnostics)
                            .next()
                            .is_some(),
                    );
                }

                if index >= 2 && !current_diagnostics.is_empty() {
                    oscillation_opportunities += 1;
                    let returned_to_prior_set = diagnostic_sets[..index - 1]
                        .iter()
                        .any(|earlier| earlier == current_diagnostics);
                    let differs_from_previous = previous_diagnostics != current_diagnostics;
                    diagnostic_oscillations +=
                        u64::from(returned_to_prior_set && differs_from_previous);
                }
            }
        }

        Self {
            version: evaluation_version(),
            runs_total: runs.len() as u64,
            runs_with_attempts,
            attempts_total,
            parse_at_1: FlowPilotRateMetric::new(parse_at_1, runs_with_attempts),
            typed_valid_at_1: FlowPilotRateMetric::new(typed_valid_at_1, runs_with_attempts),
            reconcile_valid_at_1: FlowPilotRateMetric::new(
                reconcile_valid_at_1,
                runs_with_attempts,
            ),
            success_within_1_attempt: FlowPilotRateMetric::new(
                success_within_1,
                runs_with_attempts,
            ),
            success_within_3_attempts: FlowPilotRateMetric::new(
                success_within_3,
                runs_with_attempts,
            ),
            diagnostic_regression: FlowPilotRateMetric::new(
                diagnostic_regressions,
                repair_transitions,
            ),
            repeated_diagnostic: FlowPilotRateMetric::new(repeated_diagnostics, repair_transitions),
            diagnostic_oscillation: FlowPilotRateMetric::new(
                diagnostic_oscillations,
                oscillation_opportunities,
            ),
            time_to_first_valid: FlowPilotDurationMetric::from_values(first_valid_times),
            infeasible_plan: FlowPilotRateMetric::new(infeasible_plans, assessed_plans),
            empty_board_after_run: FlowPilotRateMetric::new(
                empty_terminal_boards,
                inspected_terminal_boards,
            ),
        }
    }
}

pub fn evaluate_generation_runs(
    runs: &[FlowPilotGenerationRunRecord],
) -> FlowPilotGenerationScorecard {
    FlowPilotGenerationScorecard::from_runs(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(
        attempt_index: u32,
        elapsed_ms: u64,
        validation: (bool, bool, bool),
        accepted: bool,
        diagnostic_keys: &[&str],
    ) -> FlowPilotGenerationAttemptRecord {
        FlowPilotGenerationAttemptRecord {
            attempt_index,
            elapsed_ms,
            parse_valid: validation.0,
            typed_valid: validation.1,
            reconcile_valid: validation.2,
            accepted,
            diagnostic_keys: diagnostic_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
        }
    }

    fn run(
        run_id: &str,
        status: FlowPilotEvaluationRunStatus,
        plan_outcome: FlowPilotPlanOutcome,
        final_board_node_count: Option<usize>,
        attempts: Vec<FlowPilotGenerationAttemptRecord>,
    ) -> FlowPilotGenerationRunRecord {
        FlowPilotGenerationRunRecord {
            version: evaluation_version(),
            run_id: run_id.to_string(),
            status,
            plan_outcome,
            final_board_node_count,
            attempts,
        }
    }

    #[test]
    fn supplied_failure_pattern_surfaces_regression_repetition_and_oscillation() {
        // Mirrors the observed failure mode: broad first candidate, persistent Generic/condition
        // diagnostics, a repair that falls back to a parse failure, and then a return to the first
        // diagnostic set without ever materializing a board.
        let failed = run(
            "support-mail-agent",
            FlowPilotEvaluationRunStatus::Failed,
            FlowPilotPlanOutcome::Feasible,
            Some(0),
            vec![
                attempt(
                    1,
                    125_000,
                    (true, false, false),
                    false,
                    &["FS_TYPE_MISMATCH", "FS_UNRESOLVED_ARGUMENT"],
                ),
                attempt(
                    2,
                    245_000,
                    (false, false, false),
                    false,
                    &[
                        "FS_PARSE_ERROR",
                        "FS_UNRESOLVED_ARGUMENT",
                        "FS_UNKNOWN_INPUT_PIN",
                    ],
                ),
                attempt(
                    3,
                    360_000,
                    (true, false, false),
                    false,
                    &["FS_TYPE_MISMATCH", "FS_UNRESOLVED_ARGUMENT"],
                ),
            ],
        );

        let scorecard = evaluate_generation_runs(&[failed]);

        assert_eq!(scorecard.parse_at_1, FlowPilotRateMetric::new(1, 1));
        assert_eq!(scorecard.typed_valid_at_1, FlowPilotRateMetric::new(0, 1));
        assert_eq!(
            scorecard.success_within_3_attempts,
            FlowPilotRateMetric::new(0, 1)
        );
        assert_eq!(
            scorecard.diagnostic_regression,
            FlowPilotRateMetric::new(1, 2)
        );
        assert_eq!(
            scorecard.repeated_diagnostic,
            FlowPilotRateMetric::new(2, 2)
        );
        assert_eq!(
            scorecard.diagnostic_oscillation,
            FlowPilotRateMetric::new(1, 1)
        );
        assert_eq!(scorecard.time_to_first_valid.samples, 0);
        assert_eq!(
            scorecard.empty_board_after_run,
            FlowPilotRateMetric::new(1, 1)
        );
    }

    #[test]
    fn successful_first_typed_candidate_scores_at_one() {
        let successful = run(
            "typed-success",
            FlowPilotEvaluationRunStatus::Succeeded,
            FlowPilotPlanOutcome::Feasible,
            Some(14),
            vec![attempt(1, 1_200, (true, true, true), true, &[])],
        );

        let scorecard = evaluate_generation_runs(&[successful]);

        assert_eq!(scorecard.parse_at_1, FlowPilotRateMetric::new(1, 1));
        assert_eq!(scorecard.typed_valid_at_1, FlowPilotRateMetric::new(1, 1));
        assert_eq!(
            scorecard.reconcile_valid_at_1,
            FlowPilotRateMetric::new(1, 1)
        );
        assert_eq!(
            scorecard.success_within_1_attempt,
            FlowPilotRateMetric::new(1, 1)
        );
        assert_eq!(scorecard.time_to_first_valid.samples, 1);
        assert_eq!(scorecard.time_to_first_valid.mean_ms, Some(1_200.0));
        assert_eq!(scorecard.diagnostic_regression.denominator, 0);
        assert_eq!(scorecard.diagnostic_regression.rate, None);
        assert_eq!(
            scorecard.empty_board_after_run,
            FlowPilotRateMetric::new(0, 1)
        );
    }

    #[test]
    fn aggregate_denominators_exclude_unattempted_and_uninspectable_runs() {
        let repaired = run(
            "repaired",
            FlowPilotEvaluationRunStatus::Succeeded,
            FlowPilotPlanOutcome::Feasible,
            Some(8),
            vec![
                attempt(1, 500, (true, false, false), false, &["FS_TYPE_MISMATCH"]),
                attempt(2, 1_500, (true, true, true), true, &[]),
            ],
        );
        let infeasible = run(
            "missing-smtp-headers",
            FlowPilotEvaluationRunStatus::Failed,
            FlowPilotPlanOutcome::Infeasible,
            Some(0),
            Vec::new(),
        );
        let still_running = run(
            "still-running",
            FlowPilotEvaluationRunStatus::Running,
            FlowPilotPlanOutcome::NotAssessed,
            Some(0),
            vec![attempt(
                1,
                300,
                (false, false, false),
                false,
                &["FS_PARSE_ERROR"],
            )],
        );

        let scorecard = evaluate_generation_runs(&[repaired, infeasible, still_running]);

        assert_eq!(scorecard.runs_total, 3);
        assert_eq!(scorecard.runs_with_attempts, 2);
        assert_eq!(scorecard.attempts_total, 3);
        assert_eq!(scorecard.parse_at_1, FlowPilotRateMetric::new(1, 2));
        assert_eq!(
            scorecard.success_within_1_attempt,
            FlowPilotRateMetric::new(0, 2)
        );
        assert_eq!(
            scorecard.success_within_3_attempts,
            FlowPilotRateMetric::new(1, 2)
        );
        assert_eq!(scorecard.infeasible_plan, FlowPilotRateMetric::new(1, 2));
        assert_eq!(
            scorecard.empty_board_after_run,
            FlowPilotRateMetric::new(1, 2)
        );
        assert_eq!(scorecard.time_to_first_valid.samples, 1);
        assert_eq!(scorecard.time_to_first_valid.median_ms, Some(1_500.0));
    }

    #[test]
    fn diagnostic_keys_are_deduplicated_before_repair_metrics() {
        let duplicate_diagnostics = run(
            "deduplicated",
            FlowPilotEvaluationRunStatus::Failed,
            FlowPilotPlanOutcome::NotAssessed,
            None,
            vec![
                attempt(
                    1,
                    100,
                    (true, false, false),
                    false,
                    &["FS_TYPE_MISMATCH", "FS_TYPE_MISMATCH", ""],
                ),
                attempt(2, 200, (true, false, false), false, &["FS_TYPE_MISMATCH"]),
            ],
        );

        let scorecard = evaluate_generation_runs(&[duplicate_diagnostics]);

        assert_eq!(
            scorecard.diagnostic_regression,
            FlowPilotRateMetric::new(0, 1)
        );
        assert_eq!(
            scorecard.repeated_diagnostic,
            FlowPilotRateMetric::new(1, 1)
        );
    }

    #[test]
    fn duration_summary_uses_first_valid_attempt_and_reports_even_median() {
        let slow = run(
            "slow",
            FlowPilotEvaluationRunStatus::Succeeded,
            FlowPilotPlanOutcome::Feasible,
            Some(2),
            vec![
                attempt(1, 1_000, (true, true, false), false, &["FS_PIN"]),
                attempt(2, 4_000, (true, true, true), false, &[]),
                attempt(3, 8_000, (true, true, true), true, &[]),
            ],
        );
        let fast = run(
            "fast",
            FlowPilotEvaluationRunStatus::Succeeded,
            FlowPilotPlanOutcome::Feasible,
            Some(1),
            vec![attempt(1, 2_000, (true, true, true), true, &[])],
        );

        let scorecard = evaluate_generation_runs(&[slow, fast]);

        assert_eq!(scorecard.time_to_first_valid.samples, 2);
        assert_eq!(scorecard.time_to_first_valid.mean_ms, Some(3_000.0));
        assert_eq!(scorecard.time_to_first_valid.median_ms, Some(3_000.0));
        assert_eq!(scorecard.time_to_first_valid.min_ms, Some(2_000));
        assert_eq!(scorecard.time_to_first_valid.max_ms, Some(4_000));
    }

    #[test]
    fn records_and_scorecards_round_trip_through_json() {
        let record = run(
            "round-trip",
            FlowPilotEvaluationRunStatus::Succeeded,
            FlowPilotPlanOutcome::Feasible,
            Some(3),
            vec![attempt(1, 42, (true, true, true), true, &[])],
        );
        let encoded = serde_json::to_string(&record).expect("record should serialize");
        let decoded: FlowPilotGenerationRunRecord =
            serde_json::from_str(&encoded).expect("record should deserialize");
        assert_eq!(decoded, record);

        let scorecard = evaluate_generation_runs(&[record]);
        let encoded = serde_json::to_string(&scorecard).expect("scorecard should serialize");
        let decoded: FlowPilotGenerationScorecard =
            serde_json::from_str(&encoded).expect("scorecard should deserialize");
        assert_eq!(decoded, scorecard);
    }
}
