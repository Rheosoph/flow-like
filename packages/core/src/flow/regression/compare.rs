//! Verdict-vs-baseline comparison — never output-vs-output. The replay is
//! graded by `grade_run` (the one grader, twinned with `gradeBoardRun`); the
//! case outcome compares that verdict against the baseline verdict recorded
//! at promotion. No goldens, no stored outputs.
//!
//! Twinned with `compareToExpectation`/`errorClassOf` in
//! `packages/ui/lib/regression.ts` (the desktop suite runner grades client
//! side) — a rule change here must also land there, held together by the
//! `compare` section of `packages/core/tests/fixtures/board-test-grading.json`.

use super::grade::{LOG_QUERY_FAILED_MESSAGE, NO_METADATA_MESSAGE, RunGrade, TestVerdict};
use super::suite::FixtureBaseline;
use serde::{Deserialize, Serialize};

pub const ERROR_CLASS_ASSERT_FAIL: &str = "assert_fail";
pub const ERROR_CLASS_EXECUTION_ERROR: &str = "execution_error";
pub const ERROR_CLASS_ERROR_LOG: &str = "error_log";
pub const ERROR_CLASS_NO_METADATA: &str = "no_metadata";
pub const ERROR_CLASS_LOG_QUERY_FAILED: &str = "log_query_failed";

/// The four case outcomes:
/// - `Ok` — baseline passed, replay passes.
/// - `Regressed` — baseline passed, replay errors. **The gate signal.**
/// - `StillFailing` — baseline failed, replay fails. Neutral; a changed error
///   class is surfaced as info, never graded.
/// - `Fixed` — baseline failed, replay passes. Good news, shown as such.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CaseOutcome {
    Ok,
    Regressed,
    StillFailing { error_class_changed: bool },
    Fixed,
}

impl CaseOutcome {
    /// Whether this outcome fails the publish/promote gate.
    pub fn is_gate_failure(&self) -> bool {
        matches!(self, CaseOutcome::Regressed)
    }
}

/// Coarse classification of a failing grade, stored in the baseline at
/// promotion and compared on `StillFailing`. Deliberately structural — never
/// derived from output content: a thrown/synthesized execution error
/// outranks failed assertions, which outrank plain error-level logs. `None`
/// for a passing grade.
pub fn error_class_of(grade: &RunGrade) -> Option<String> {
    match grade.verdict {
        TestVerdict::Pass => None,
        TestVerdict::Error => Some(
            match grade.execution_error.as_deref() {
                Some(NO_METADATA_MESSAGE) => ERROR_CLASS_NO_METADATA,
                Some(LOG_QUERY_FAILED_MESSAGE) => ERROR_CLASS_LOG_QUERY_FAILED,
                _ => ERROR_CLASS_EXECUTION_ERROR,
            }
            .to_string(),
        ),
        TestVerdict::Fail => Some(
            if grade.execution_error.is_some() {
                ERROR_CLASS_EXECUTION_ERROR
            } else if grade.assert_fail > 0 {
                ERROR_CLASS_ASSERT_FAIL
            } else {
                ERROR_CLASS_ERROR_LOG
            }
            .to_string(),
        ),
    }
}

/// Compare a replay's grade against the fixture's recorded baseline. Any
/// non-`Pass` verdict counts as failing on both sides — an ungradable replay
/// (`Error`) of a passing baseline is a regression, never a shrug. Authored
/// tests compare against [`FixtureBaseline::pass_expectation`].
pub fn compare_to_expectation(baseline: &FixtureBaseline, grade: &RunGrade) -> CaseOutcome {
    let baseline_failed = baseline.verdict != TestVerdict::Pass;
    let replay_failed = grade.verdict != TestVerdict::Pass;
    match (baseline_failed, replay_failed) {
        (false, false) => CaseOutcome::Ok,
        (false, true) => CaseOutcome::Regressed,
        (true, true) => CaseOutcome::StillFailing {
            error_class_changed: baseline.error_class != error_class_of(grade),
        },
        (true, false) => CaseOutcome::Fixed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::regression::grade::{RunGradeEvidence, grade_run};

    fn baseline(verdict: TestVerdict, error_class: Option<&str>) -> FixtureBaseline {
        FixtureBaseline {
            verdict,
            error_class: error_class.map(str::to_string),
            visited_node_ids: vec![],
            recorded_at: 0,
        }
    }

    fn passing_grade() -> RunGrade {
        grade_run(RunGradeEvidence {
            metadata_present: true,
            ..RunGradeEvidence::default()
        })
    }

    fn assert_fail_grade() -> RunGrade {
        grade_run(RunGradeEvidence {
            metadata_present: true,
            assert_logs: vec!["ASSERT_FAIL expected 1 got 2".to_string()],
            ..RunGradeEvidence::default()
        })
    }

    fn execution_error_grade() -> RunGrade {
        grade_run(RunGradeEvidence {
            metadata_present: true,
            execution_error: Some("boom".to_string()),
            ..RunGradeEvidence::default()
        })
    }

    #[test]
    fn all_four_quadrants() {
        let pass_baseline = baseline(TestVerdict::Pass, None);
        let fail_baseline = baseline(TestVerdict::Fail, Some(ERROR_CLASS_ASSERT_FAIL));

        assert_eq!(
            compare_to_expectation(&pass_baseline, &passing_grade()),
            CaseOutcome::Ok
        );
        assert_eq!(
            compare_to_expectation(&pass_baseline, &assert_fail_grade()),
            CaseOutcome::Regressed
        );
        assert_eq!(
            compare_to_expectation(&fail_baseline, &assert_fail_grade()),
            CaseOutcome::StillFailing {
                error_class_changed: false
            }
        );
        assert_eq!(
            compare_to_expectation(&fail_baseline, &passing_grade()),
            CaseOutcome::Fixed
        );
    }

    #[test]
    fn still_failing_flags_an_error_class_change() {
        let fail_baseline = baseline(TestVerdict::Fail, Some(ERROR_CLASS_ASSERT_FAIL));
        assert_eq!(
            compare_to_expectation(&fail_baseline, &execution_error_grade()),
            CaseOutcome::StillFailing {
                error_class_changed: true
            }
        );
    }

    #[test]
    fn ungradable_replay_of_a_passing_baseline_is_a_regression() {
        let pass_baseline = baseline(TestVerdict::Pass, None);
        let no_metadata = grade_run(RunGradeEvidence::default());
        assert_eq!(no_metadata.verdict, TestVerdict::Error);
        assert_eq!(
            compare_to_expectation(&pass_baseline, &no_metadata),
            CaseOutcome::Regressed
        );
        assert!(compare_to_expectation(&pass_baseline, &no_metadata).is_gate_failure());
    }

    #[test]
    fn error_baselines_count_as_failing() {
        let error_baseline = baseline(TestVerdict::Error, Some(ERROR_CLASS_LOG_QUERY_FAILED));
        assert_eq!(
            compare_to_expectation(&error_baseline, &passing_grade()),
            CaseOutcome::Fixed
        );
        let unreadable = grade_run(RunGradeEvidence {
            metadata_present: true,
            log_query_failed: true,
            ..RunGradeEvidence::default()
        });
        assert_eq!(
            compare_to_expectation(&error_baseline, &unreadable),
            CaseOutcome::StillFailing {
                error_class_changed: false
            }
        );
    }

    #[test]
    fn error_class_taxonomy() {
        assert_eq!(error_class_of(&passing_grade()), None);
        assert_eq!(
            error_class_of(&assert_fail_grade()).as_deref(),
            Some(ERROR_CLASS_ASSERT_FAIL)
        );
        assert_eq!(
            error_class_of(&execution_error_grade()).as_deref(),
            Some(ERROR_CLASS_EXECUTION_ERROR)
        );

        let error_log_only = grade_run(RunGradeEvidence {
            metadata_present: true,
            error_logs: vec!["exploded".to_string()],
            ..RunGradeEvidence::default()
        });
        assert_eq!(
            error_class_of(&error_log_only).as_deref(),
            Some(ERROR_CLASS_ERROR_LOG)
        );

        let no_metadata = grade_run(RunGradeEvidence::default());
        assert_eq!(
            error_class_of(&no_metadata).as_deref(),
            Some(ERROR_CLASS_NO_METADATA)
        );

        let unreadable = grade_run(RunGradeEvidence {
            metadata_present: true,
            log_query_failed: true,
            ..RunGradeEvidence::default()
        });
        assert_eq!(
            error_class_of(&unreadable).as_deref(),
            Some(ERROR_CLASS_LOG_QUERY_FAILED)
        );

        // Execution error outranks failed assertions within a Fail verdict.
        let both = grade_run(RunGradeEvidence {
            metadata_present: true,
            assert_logs: vec!["ASSERT_FAIL x".to_string()],
            execution_error: Some("boom".to_string()),
            ..RunGradeEvidence::default()
        });
        assert_eq!(
            error_class_of(&both).as_deref(),
            Some(ERROR_CLASS_EXECUTION_ERROR)
        );
    }

    #[test]
    fn authored_tests_use_the_pass_expectation() {
        let expectation = FixtureBaseline::pass_expectation(42);
        assert_eq!(expectation.verdict, TestVerdict::Pass);
        assert_eq!(
            compare_to_expectation(&expectation, &assert_fail_grade()),
            CaseOutcome::Regressed
        );
    }
}
