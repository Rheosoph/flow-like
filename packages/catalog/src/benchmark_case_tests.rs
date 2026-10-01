use flow_like::flow::{
    ast::apply_board_commands_to_board,
    board::{Board, LayerType},
    copilot::{
        CheckFlowScriptArgs, FlowIrDraftMode, FlowIrDraftStore, WriteFlowScriptArgs,
        behavioral_evaluation::{WorkflowBenchmarkCase, WorkflowBenchmarkCheck},
        benchmark_cases::workflow_benchmark_cases,
        node_to_metadata,
    },
};
use flow_like_storage::Path;
use flow_like_types::tokio;

use crate::draft_test::{DraftTestResult, DraftTestStatus, prepare_and_test_draft_board};

async fn run_fixture_source(
    case: &WorkflowBenchmarkCase,
    source: &str,
    check: &WorkflowBenchmarkCheck,
) -> Result<DraftTestResult, String> {
    let board = Board::new_detached(
        Some(format!("benchmark-{}", case.id)),
        Path::from("benchmark-tests"),
    );
    prepare_and_test_draft_board(
        board,
        &check.entry,
        check.payload.clone(),
        |mut board, state| async move {
            let catalog = state
                .node_registry
                .read()
                .await
                .get_nodes()
                .map_err(|error| error.to_string())?;
            let metadata: Vec<_> = catalog.iter().map(node_to_metadata).collect();
            let store = FlowIrDraftStore::new();
            let binding = store.bind_request_acceptance_contract(&board.id, case.public_prompt());
            let written = store.write_flowscript_with_acceptance_binding(
                &board,
                &metadata,
                WriteFlowScriptArgs {
                    draft_id: "fixture".into(),
                    replace_existing: false,
                    mode: FlowIrDraftMode::Additive,
                    source: source.into(),
                    edits: Vec::new(),
                    allow_scope_reduction: false,
                },
                &binding,
            );
            if written.revision != Some(0) {
                return Err(format!("source was not retained: {written:?}"));
            }
            let snapshot = store
                .prepare_flowscript_test(
                    &board,
                    &metadata,
                    CheckFlowScriptArgs {
                        draft_id: "fixture".into(),
                        expected_revision: 0,
                    },
                    &binding,
                )
                .map_err(|error| format!("source check failed: {error:?}"))?;
            if snapshot.revision != 0 || snapshot.source_fingerprint.is_empty() {
                return Err("the checked fixture lacks exact source evidence".into());
            }
            let applied =
                apply_board_commands_to_board(&mut board, snapshot.commands, &catalog, state, None)
                    .await
                    .map_err(|error| error.to_string())?;
            if !applied.diagnostics.is_empty() {
                return Err(applied.diagnostics.join("; "));
            }
            for name in &case.required_functions {
                if !board
                    .layers
                    .values()
                    .any(|layer| matches!(layer.r#type, LayerType::Function) && layer.name == *name)
                {
                    return Err(format!("compiled fixture lacks required function {name}"));
                }
            }
            Ok(board)
        },
    )
    .await
}

#[tokio::test]
async fn benchmark_reference_sources_pass_all_hidden_checks() {
    let cases = workflow_benchmark_cases();
    assert_eq!(cases.len(), 16);
    assert_eq!(
        cases.iter().map(|case| case.checks.len()).sum::<usize>(),
        64
    );
    let mut failures = Vec::new();
    for case in cases {
        case.validate().unwrap();
        assert!((3..=5).contains(&case.checks.len()), "{}", case.id);
        for check in &case.checks {
            match run_fixture_source(&case, &case.reference_source, check).await {
                Ok(result)
                    if result.status == DraftTestStatus::Success
                        && result.errors.is_empty()
                        && result.outputs == vec![check.expected_output.clone()]
                        && result.output.as_ref() == Some(&check.expected_output) => {}
                result => failures.push(format!("{}/{}: {result:?}", case.id, check.id)),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn benchmark_repair_seeds_run_but_fail_behavioral_grading() {
    let mut repairs = 0;
    let mut failures = Vec::new();
    for case in workflow_benchmark_cases() {
        let Some(source) = &case.initial_source else {
            continue;
        };
        repairs += 1;
        let mut behavioral_failures = 0;
        let mut preserved_entries = 0;
        for check in &case.checks {
            match run_fixture_source(&case, source, check).await {
                Ok(result)
                    if result.status == DraftTestStatus::Success
                        && result.errors.is_empty()
                        && result.outputs.len() == 1 =>
                {
                    let passed = result.output.as_ref() == Some(&check.expected_output);
                    if check.id.starts_with("preserve-") {
                        preserved_entries += 1;
                        if !passed {
                            failures.push(format!(
                                "{}/{}: seed changed the preservation sentinel: {result:?}",
                                case.id, check.id
                            ));
                        }
                    } else if !passed {
                        behavioral_failures += 1;
                    }
                }
                result => failures.push(format!(
                    "{}/{}: repair seed must compile and execute: {result:?}",
                    case.id, check.id
                )),
            }
        }
        if behavioral_failures == 0 {
            failures.push(format!(
                "{}: repair seed has no behavioral failure",
                case.id
            ));
        }
        if preserved_entries == 0 {
            failures.push(format!(
                "{}: repair seed has no preserved entry check",
                case.id
            ));
        }
    }
    assert_eq!(repairs, 4);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
