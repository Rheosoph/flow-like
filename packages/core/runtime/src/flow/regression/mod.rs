//! One grader + one discovery for board tests, twinned with
//! `packages/ui/lib/board-tests.ts`. Both sides are held to the shared
//! conformance fixture `packages/core/tests/fixtures/board-test-grading.json` —
//! a deliberate rule change on either side must update the fixture and thereby
//! fail the other side's test.

pub mod compare;
pub mod discover;
pub mod grade;
pub mod selection;
pub mod suite;

pub use compare::{
    CaseOutcome, ERROR_CLASS_ASSERT_FAIL, ERROR_CLASS_ERROR_LOG, ERROR_CLASS_EXECUTION_ERROR,
    ERROR_CLASS_LOG_QUERY_FAILED, ERROR_CLASS_NO_METADATA, compare_to_expectation, error_class_of,
};
pub use discover::{
    BoardTestCase, discover_board_tests, event_alias_of, is_test_event_alias, is_test_event_node,
};
pub use grade::{
    LOG_QUERY_FAILED_MESSAGE, NO_METADATA_MESSAGE, RunGrade, RunGradeEvidence, TestVerdict,
    grade_run,
};
pub use selection::{
    CORPUS_SCAN_CAP, CORPUS_WINDOWS, CorpusCandidate, CorpusSelection, REPLAY_EXCLUSION_SUITE_RUNS,
    dedupe_by_run_id, dedupe_by_shape, filter_excluded, refine_corpus_rows, select_corpus_window,
    shape_hash, stratify_failures,
};
pub use suite::{
    AUTHORED_TESTS_CAP, CAVEAT_CALLER_OAUTH_TOKENS, CAVEAT_GRADING_BLIND, DESKTOP_RUN_ARCHIVE_CAP,
    FIXTURE_PAYLOAD_CAP_BYTES, FixtureBaseline, GateMode, PAYLOAD_PREVIEW_CAP_BYTES,
    RegressionFixture, RegressionSuite, SUITE_CASE_CAP, SuiteCase, SuiteCasePlan,
    drop_raw_body_duplicates, key_requires_redaction, payload_preview, plan_suite_cases,
    prepare_fixture_payload, redact_by_key_name,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::{Board, ExecutionMode, ExecutionStage};
    use crate::flow::execution::LogLevel;
    use crate::flow::node::Node;
    use flow_like_storage::Path;
    use serde::Deserialize;
    use std::collections::HashMap;
    use std::time::SystemTime;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/board-test-grading.json");

    #[derive(Deserialize)]
    struct Fixture {
        grading: Vec<GradingCase>,
        discovery: Vec<DiscoveryCase>,
        compare: Vec<CompareCase>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GradingCase {
        case: String,
        metadata: bool,
        assert_logs: Vec<String>,
        error_logs: Vec<String>,
        execution_error: Option<String>,
        log_query_failed: bool,
        expect: GradeExpect,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GradeExpect {
        verdict: String,
        assert_ok: usize,
        assert_fail: usize,
        execution_error: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct DiscoveryCase {
        case: String,
        name: String,
        friendly_name: Option<String>,
        start: bool,
        expect: DiscoveryExpect,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct DiscoveryExpect {
        is_test: bool,
        alias: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CompareCase {
        case: String,
        baseline: CompareBaseline,
        replay: CompareReplay,
        expect: CompareExpect,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CompareBaseline {
        verdict: TestVerdict,
        error_class: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CompareReplay {
        metadata: bool,
        assert_logs: Vec<String>,
        error_logs: Vec<String>,
        execution_error: Option<String>,
        log_query_failed: bool,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CompareExpect {
        outcome: String,
        /// `Some` only for `still_failing` outcomes.
        error_class_changed: Option<bool>,
        gate_failure: bool,
        replay_error_class: Option<String>,
    }

    fn fixture() -> Fixture {
        flow_like_types::json::from_str(FIXTURE)
            .expect("board-test-grading.json must parse — the TS twin reads the same file")
    }

    fn board_with(nodes: Vec<Node>) -> Board {
        Board {
            id: "board".to_string(),
            name: "Board".to_string(),
            description: String::new(),
            nodes: nodes
                .into_iter()
                .map(|node| (node.id.clone(), node))
                .collect(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
            pin_index: None,
        }
    }

    fn event_node(id: &str, name: &str, friendly_name: &str, start: bool) -> Node {
        let mut node = Node::new(name, friendly_name, "", "events");
        node.id = id.to_string();
        node.start = Some(start);
        node
    }

    #[test]
    fn grading_conformance_fixture() {
        let fixture = fixture();
        assert!(!fixture.grading.is_empty());
        for case in fixture.grading {
            let grade = grade_run(RunGradeEvidence {
                metadata_present: case.metadata,
                assert_logs: case.assert_logs,
                error_logs: case.error_logs,
                execution_error: case.execution_error,
                log_query_failed: case.log_query_failed,
            });
            assert_eq!(
                grade.verdict.as_str(),
                case.expect.verdict,
                "verdict for grading case '{}'",
                case.case
            );
            assert_eq!(
                grade.assert_ok, case.expect.assert_ok,
                "assert_ok for grading case '{}'",
                case.case
            );
            assert_eq!(
                grade.assert_fail, case.expect.assert_fail,
                "assert_fail for grading case '{}'",
                case.case
            );
            assert_eq!(
                grade.execution_error, case.expect.execution_error,
                "execution_error for grading case '{}'",
                case.case
            );
        }
    }

    #[test]
    fn discovery_conformance_fixture() {
        let fixture = fixture();
        assert!(!fixture.discovery.is_empty());
        for case in fixture.discovery {
            let node = event_node(
                "node-1",
                &case.name,
                case.friendly_name.as_deref().unwrap_or(""),
                case.start,
            );
            assert_eq!(
                event_alias_of(&node),
                case.expect.alias,
                "alias for discovery case '{}'",
                case.case
            );
            assert_eq!(
                is_test_event_node(&node),
                case.expect.is_test,
                "is_test for discovery case '{}'",
                case.case
            );
            let discovered = discover_board_tests(&board_with(vec![node]));
            if case.expect.is_test {
                assert_eq!(
                    discovered,
                    vec![BoardTestCase {
                        node_id: "node-1".to_string(),
                        alias: case.expect.alias.clone(),
                    }],
                    "discover_board_tests for discovery case '{}'",
                    case.case
                );
            } else {
                assert!(
                    discovered.is_empty(),
                    "discover_board_tests must skip discovery case '{}'",
                    case.case
                );
            }
        }
    }

    #[test]
    fn discovery_collects_only_tests_and_sorts_by_alias() {
        let board = board_with(vec![
            event_node("a", "events_simple", "Test Zeta", true),
            event_node("b", "events_simple", "testAlpha", true),
            event_node("c", "events_simple", "dashboardLoad", true),
            event_node("d", "events_simple", "testHidden", false),
        ]);
        let aliases: Vec<String> = discover_board_tests(&board)
            .into_iter()
            .map(|case| case.alias)
            .collect();
        assert_eq!(aliases, vec!["testAlpha", "testZeta"]);
    }

    #[test]
    fn compare_conformance_fixture() {
        let fixture = fixture();
        assert!(!fixture.compare.is_empty());
        for case in fixture.compare {
            let baseline = suite::FixtureBaseline {
                verdict: case.baseline.verdict,
                error_class: case.baseline.error_class,
                visited_node_ids: vec![],
                recorded_at: 0,
            };
            let grade = grade_run(RunGradeEvidence {
                metadata_present: case.replay.metadata,
                assert_logs: case.replay.assert_logs,
                error_logs: case.replay.error_logs,
                execution_error: case.replay.execution_error,
                log_query_failed: case.replay.log_query_failed,
            });
            assert_eq!(
                error_class_of(&grade),
                case.expect.replay_error_class,
                "replay error class for compare case '{}'",
                case.case
            );
            let outcome = compare_to_expectation(&baseline, &grade);
            let (label, error_class_changed) = match outcome {
                CaseOutcome::Ok => ("ok", None),
                CaseOutcome::Regressed => ("regressed", None),
                CaseOutcome::StillFailing {
                    error_class_changed,
                } => ("still_failing", Some(error_class_changed)),
                CaseOutcome::Fixed => ("fixed", None),
            };
            assert_eq!(
                label, case.expect.outcome,
                "outcome for compare case '{}'",
                case.case
            );
            assert_eq!(
                error_class_changed, case.expect.error_class_changed,
                "error_class_changed for compare case '{}'",
                case.case
            );
            assert_eq!(
                outcome.is_gate_failure(),
                case.expect.gate_failure,
                "gate failure for compare case '{}'",
                case.case
            );
        }
    }

    #[test]
    fn grader_pins_the_twinned_messages() {
        let no_metadata = grade_run(RunGradeEvidence::default());
        assert_eq!(no_metadata.verdict, TestVerdict::Error);
        assert_eq!(
            no_metadata.execution_error.as_deref(),
            Some(NO_METADATA_MESSAGE)
        );

        let unreadable = grade_run(RunGradeEvidence {
            metadata_present: true,
            log_query_failed: true,
            ..RunGradeEvidence::default()
        });
        assert_eq!(unreadable.verdict, TestVerdict::Error);
        assert_eq!(
            unreadable.execution_error.as_deref(),
            Some(LOG_QUERY_FAILED_MESSAGE)
        );
    }
}
