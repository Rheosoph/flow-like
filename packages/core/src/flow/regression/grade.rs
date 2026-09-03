use serde::{Deserialize, Serialize};

pub const NO_METADATA_MESSAGE: &str =
    "The run returned no metadata, so its logs could not be graded.";
pub const LOG_QUERY_FAILED_MESSAGE: &str =
    "The run's logs could not be queried, so the run could not be graded.";

/// Everything the verdict rule looks at, gathered by an execute adapter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunGradeEvidence {
    /// Whether the run resolved with log metadata at all — a run without it
    /// cannot be graded and is an error, never a pass.
    pub metadata_present: bool,
    /// Messages of the logs matching `message LIKE 'ASSERT_%'`.
    pub assert_logs: Vec<String>,
    /// Messages of the logs matching `log_level >= 3`.
    pub error_logs: Vec<String>,
    pub execution_error: Option<String>,
    /// True when the run's log store could not be queried. `false` preserves
    /// the interactive degrade-to-pass behavior; a regression suite sets it so
    /// an unreadable log store yields [`TestVerdict::Error`], never a green light.
    pub log_query_failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestVerdict {
    Pass,
    Fail,
    Error,
}

impl TestVerdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestVerdict::Pass => "pass",
            TestVerdict::Fail => "fail",
            TestVerdict::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunGrade {
    pub verdict: TestVerdict,
    pub assert_ok: usize,
    pub assert_fail: usize,
    pub failed_assertions: Vec<String>,
    /// The caller's execution error, or a synthesized one naming why the run
    /// was ungradable.
    pub execution_error: Option<String>,
}

/// The one verdict rule: `ASSERT_FAIL` markers, error-level logs or a thrown
/// execution fail the run; a run without metadata (or, when `log_query_failed`
/// is set, without readable logs) is an error, never a pass.
///
/// Twinned with `gradeBoardRun` in `packages/ui/lib/board-tests.ts` — a rule
/// change here must also land there, held together by
/// `packages/core/tests/fixtures/board-test-grading.json`.
pub fn grade_run(evidence: RunGradeEvidence) -> RunGrade {
    let assert_ok = evidence
        .assert_logs
        .iter()
        .filter(|message| message.starts_with("ASSERT_OK"))
        .count();
    let failed_assertions: Vec<String> = evidence
        .assert_logs
        .iter()
        .filter(|message| message.starts_with("ASSERT_FAIL"))
        .cloned()
        .collect();
    let mut execution_error = evidence.execution_error;
    let has_failures = !failed_assertions.is_empty()
        || !evidence.error_logs.is_empty()
        || execution_error.is_some();
    if !evidence.metadata_present && execution_error.is_none() {
        execution_error = Some(NO_METADATA_MESSAGE.to_string());
    } else if evidence.log_query_failed && execution_error.is_none() {
        execution_error = Some(LOG_QUERY_FAILED_MESSAGE.to_string());
    }
    let verdict = if !evidence.metadata_present || evidence.log_query_failed {
        TestVerdict::Error
    } else if has_failures {
        TestVerdict::Fail
    } else {
        TestVerdict::Pass
    };
    RunGrade {
        verdict,
        assert_ok,
        assert_fail: failed_assertions.len(),
        failed_assertions,
        execution_error,
    }
}
