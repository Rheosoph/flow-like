//! Isolated output checks for exact retained FlowScript revisions.

use std::sync::Arc;

use rig::{completion::ToolDefinition, tool::Tool};
use schemars::schema_for;
use serde_json::{Value, json};

use super::ir_tools::{
    CheckFlowScriptArgs, FlowIrAcceptanceBinding, FlowIrDraftStore, TestFlowScriptArgs,
};
use super::provider::CatalogProvider;
use super::tools::FlowScriptToolError;
use crate::flow::board::Board;

const MAX_TEST_JSON_BYTES: usize = 16_384;

pub struct TestFlowScriptTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
    pub store: Arc<FlowIrDraftStore>,
    pub acceptance_binding: FlowIrAcceptanceBinding,
}

impl TestFlowScriptTool {
    /// Shared entry point for the core loop and SDK adapters. No live board writes occur here.
    pub async fn run(&self, args: TestFlowScriptArgs) -> Result<String, FlowScriptToolError> {
        if args.entry.trim().is_empty()
            || args.entry.len() > 256
            || serde_json::to_vec(&args.payload)
                .map_or(true, |bytes| bytes.len() > MAX_TEST_JSON_BYTES)
            || serde_json::to_vec(&args.expected_output)
                .map_or(true, |bytes| bytes.len() > MAX_TEST_JSON_BYTES)
        {
            return Ok(json!({
                "status": "blocked", "code": "FLOWSCRIPT_TEST_INPUT_LIMIT",
                "message": "Provide a named entry of at most 256 bytes and payload/expected output of at most 16 KiB each.",
                "passed": false,
            }).to_string());
        }
        let catalog = self.provider.get_all_metadata().await;
        self.store.observe_board(&self.board);
        let snapshot = match self.store.prepare_flowscript_test(
            &self.board,
            &catalog,
            CheckFlowScriptArgs {
                draft_id: args.draft_id,
                expected_revision: args.expected_revision,
            },
            &self.acceptance_binding,
        ) {
            Ok(snapshot) => snapshot,
            Err(response) => {
                return Ok(json!({
                    "status": "blocked", "passed": false, "applied": false,
                    "code": response.code.unwrap_or_else(|| "FLOWSCRIPT_TEST_NOT_READY".into()),
                    "message": response.message, "draft_id": response.draft_id,
                    "revision": response.revision, "compiler_status": response.status,
                    "validation_sequence": response.validation_sequence,
                    "base_fingerprint": response.base_fingerprint,
                    "source_fingerprint": response.source_fingerprint,
                    "catalog_fingerprint": response.catalog_fingerprint,
                    "commands_fingerprint": response.commands_fingerprint,
                    "diagnostics": response.diagnostics,
                })
                .to_string());
            }
        };
        let payload_fingerprint = blake3::hash(
            &serde_json::to_vec(&args.payload)
                .map_err(|error| FlowScriptToolError(error.to_string()))?,
        )
        .to_hex()
        .to_string();
        let runtime = self
            .provider
            .test_draft_board(
                self.board.as_ref().clone(),
                snapshot.commands.clone(),
                args.entry.clone(),
                args.payload,
            )
            .await;
        let mut receipt = json!({
            "schema": "flowpilot.flowscript-draft-test/v1",
            "draft_id": snapshot.draft_id,
            "revision": snapshot.revision,
            "validation_sequence": snapshot.validation_sequence,
            "board_id": self.board.id,
            "base_fingerprint": snapshot.base_fingerprint,
            "source_fingerprint": snapshot.source_fingerprint,
            "catalog_fingerprint": snapshot.catalog_fingerprint,
            "commands_fingerprint": snapshot.commands_fingerprint,
            "payload_fingerprint": payload_fingerprint,
            "entry": args.entry,
            "expected_output": args.expected_output,
            "passed": false,
            "applied": false,
            "certification": "draft_output_only",
        });
        match runtime {
            Err(error) => {
                receipt["status"] = json!("blocked");
                receipt["code"] = json!("FLOWSCRIPT_TEST_UNAVAILABLE");
                receipt["message"] = json!(error.chars().take(2000).collect::<String>());
            }
            Ok(result) => {
                let passed = runtime_matches_expected(&result, &args.expected_output);
                receipt["status"] = json!(if passed { "success" } else { "failed" });
                receipt["passed"] = json!(passed);
                receipt["message"] = json!(if passed {
                    "The isolated draft returned the expected result for this input. This does not verify app integration or other inputs."
                } else {
                    "The isolated draft did not return exactly one successful result matching the expected output. Repair the retained source and test its new revision."
                });
                receipt["runtime"] = result;
            }
        }
        let current_catalog = self.provider.get_all_metadata().await;
        if !self.store.flowscript_test_is_current(
            &self.board,
            &current_catalog,
            &snapshot,
            &self.acceptance_binding,
        ) {
            receipt["status"] = json!("stale");
            receipt["passed"] = json!(false);
            receipt["message"] = json!(
                "The retained draft changed during testing. This observation belongs to the captured revision; test the current revision again."
            );
        }
        Ok(receipt.to_string())
    }
}

fn runtime_matches_expected(runtime: &Value, expected: &Value) -> bool {
    runtime.get("status").and_then(Value::as_str) == Some("success")
        && runtime
            .get("outputs")
            .and_then(Value::as_array)
            .is_some_and(|outputs| {
                outputs.len() == 1
                    && outputs[0] == *expected
                    && runtime.get("output") == Some(&outputs[0])
            })
}

impl Tool for TestFlowScriptTool {
    const NAME: &'static str = "test_flowscript";
    type Error = FlowScriptToolError;
    type Args = TestFlowScriptArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Execute an exact retained FlowScript revision on a disposable board and compare its single Generic Event result with expected_output. Use a small deterministic transformation with a named entry and fixture payload before commit; a clean draft is checked internally. The host uses a restricted built-in catalog and fresh memory stores, without live app credentials or external services. Unsupported nodes are reported as blocked, never executed on the live board. Derive expected_output from the requested behavior before testing; repair source rather than weakening the expectation to obtain a pass. Results certify only this input/output case at this revision, not the complete app. This tool never applies or queues the draft.".to_string(),
            parameters: serde_json::to_value(schema_for!(TestFlowScriptArgs))
                .unwrap_or_else(|_| json!({ "type": "object" })),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.run(args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::copilot::{
        BoardCommand, NodeMetadata, PatchFlowScriptArgs, WriteFlowScriptArgs, node_to_metadata,
    };
    use crate::flow::node::Node;
    use crate::flow::variable::VariableType;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    const SOURCE: &str = "eventsGeneric testCase() {\n    return \"before\"\n}";

    struct FakeProvider {
        catalog: Vec<NodeMetadata>,
        live_catalog: std::sync::Mutex<Option<Vec<NodeMetadata>>>,
        result: Value,
        calls: AtomicUsize,
        pause: bool,
        started: Notify,
        resume: Notify,
    }

    #[async_trait::async_trait]
    impl CatalogProvider for FakeProvider {
        async fn search(&self, _: &str) -> Vec<NodeMetadata> {
            self.catalog.clone()
        }
        async fn search_by_pin_type(&self, _: &str, _: bool) -> Vec<NodeMetadata> {
            self.catalog.clone()
        }
        async fn filter_by_category(&self, _: &str) -> Vec<NodeMetadata> {
            self.catalog.clone()
        }
        async fn get_node_metadata(&self, name: &str) -> Option<NodeMetadata> {
            self.catalog.iter().find(|node| node.name == name).cloned()
        }
        async fn get_all_nodes(&self) -> Vec<String> {
            self.catalog.iter().map(|node| node.name.clone()).collect()
        }
        async fn get_all_metadata(&self) -> Vec<NodeMetadata> {
            self.live_catalog
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| self.catalog.clone())
        }
        async fn test_draft_board(
            &self,
            board: Board,
            commands: Vec<BoardCommand>,
            entry: String,
            payload: Option<Value>,
        ) -> Result<Value, String> {
            assert!(
                board.nodes.is_empty(),
                "the provider must receive the unmodified base board"
            );
            assert!(
                !commands.is_empty(),
                "the provider must receive checked materialization commands"
            );
            assert_eq!(entry, "testCase");
            assert_eq!(payload, Some(json!({"name": "Ada"})));
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.pause {
                self.started.notify_one();
                self.resume.notified().await;
            }
            Ok(self.result.clone())
        }
    }

    fn fixture(
        source: &str,
        result: Value,
        pause: bool,
    ) -> (TestFlowScriptTool, Arc<FakeProvider>) {
        let mut entry = Node::new("events_generic", "Generic Event", "", "Events");
        entry.set_start(true);
        entry.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let mut output = Node::new(
            "events_generic_return_result",
            "Return Generic Result",
            "",
            "Events",
        );
        output.add_input_pin("exec_in", "In", "", VariableType::Execution);
        output.add_input_pin("response", "Result", "", VariableType::Generic);
        let provider = Arc::new(FakeProvider {
            catalog: vec![node_to_metadata(&entry), node_to_metadata(&output)],
            live_catalog: std::sync::Mutex::new(None),
            result,
            calls: AtomicUsize::new(0),
            pause,
            started: Notify::new(),
            resume: Notify::new(),
        });
        let board = Arc::new(Board::new_detached(
            None,
            flow_like_storage::Path::default(),
        ));
        let store = Arc::new(FlowIrDraftStore::default());
        let acceptance_binding =
            store.bind_request_acceptance_contract(&board.id, "Return a result.");
        let written = store.write_flowscript_with_acceptance_binding(
            &board,
            &provider.catalog,
            serde_json::from_value::<WriteFlowScriptArgs>(
                json!({"draft_id": "fixture", "source": source}),
            )
            .unwrap(),
            &acceptance_binding,
        );
        assert_eq!(written.revision, Some(0), "{written:?}");
        if source == SOURCE {
            assert!(written.diagnostics.is_empty(), "{written:?}");
        }
        (
            TestFlowScriptTool {
                board,
                provider: provider.clone(),
                store,
                acceptance_binding,
            },
            provider,
        )
    }

    fn args(expected_output: Value) -> TestFlowScriptArgs {
        TestFlowScriptArgs {
            draft_id: "fixture".into(),
            expected_revision: 0,
            entry: "testCase".into(),
            payload: Some(json!({"name": "Ada"})),
            expected_output,
        }
    }

    async fn run(tool: &TestFlowScriptTool, expected: Value) -> Value {
        serde_json::from_str(&tool.run(args(expected)).await.unwrap()).unwrap()
    }

    #[test]
    fn draft_output_check_requires_one_successful_runtime_result() {
        let expected = json!({"label": "Ada", "count": 1});
        let mut runtime = json!({"status": "success", "output": expected, "outputs": [expected]});
        assert!(runtime_matches_expected(&runtime, &expected));
        assert!(!runtime_matches_expected(
            &runtime,
            &json!({"label": "Ada", "count": 2})
        ));
        runtime["status"] = json!("failed");
        assert!(!runtime_matches_expected(&runtime, &expected));
        runtime["status"] = json!("success");
        runtime["outputs"] = json!([expected, expected]);
        assert!(!runtime_matches_expected(&runtime, &expected));
        assert!(!runtime_matches_expected(
            &json!({"status": "success", "output": null}),
            &Value::Null
        ));
        assert!(runtime_matches_expected(
            &json!({"status": "success", "output": null, "outputs": [null]}),
            &Value::Null
        ));
    }

    #[test]
    fn draft_test_arguments_require_an_expectation_and_cannot_supply_source() {
        let args = json!({"draft_id": "draft", "expected_revision": 0, "entry": "normalize", "expected_output": null});
        assert!(serde_json::from_value::<TestFlowScriptArgs>(args.clone()).is_ok());
        let mut missing = args.clone();
        missing.as_object_mut().unwrap().remove("expected_output");
        assert!(serde_json::from_value::<TestFlowScriptArgs>(missing).is_err());
        let mut source = args;
        source["source"] = json!("replacement source");
        assert!(serde_json::from_value::<TestFlowScriptArgs>(source).is_err());
        let schema = serde_json::to_value(schema_for!(TestFlowScriptArgs)).unwrap();
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("expected_output"))
        );
        for field in ["payload", "expected_output"] {
            let property = &schema["properties"][field];
            let types = property["type"].as_array().unwrap();
            let description = property["description"].as_str().unwrap();
            for json_type in ["object", "array", "string", "number", "boolean", "null"] {
                assert!(types.contains(&json!(json_type)), "{field}: {schema}");
                assert!(description.contains(json_type), "{field}: {description}");
            }
            let nested = property["items"]["items"]["type"].as_array().unwrap();
            assert!(nested.contains(&json!("array")), "{field}: {schema}");
        }
    }

    #[tokio::test]
    async fn draft_tool_compares_exact_expectations_including_null_without_queueing() {
        for expected in [json!({"label": "Ada", "count": 1}), Value::Null] {
            let (tool, provider) = fixture(
                SOURCE,
                json!({"status": "success", "output": expected, "outputs": [expected]}),
                false,
            );
            let receipt = run(&tool, expected.clone()).await;
            assert_eq!(receipt["passed"], true, "{receipt}");
            assert_eq!(receipt["status"], "success");
            assert_eq!(receipt["expected_output"], expected);
            assert_eq!(receipt["revision"], 0);
            assert_eq!(receipt["applied"], false);
            assert!(tool.board.nodes.is_empty());
            assert!(
                tool.store
                    .latest_pending_commit_token(&tool.board.id)
                    .is_none()
            );

            let mismatch = run(&tool, json!({"different": "expectation"})).await;
            assert_eq!(mismatch["status"], "failed", "{mismatch}");
            assert_eq!(mismatch["passed"], false);
            assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
        }
    }

    #[tokio::test]
    async fn draft_tool_does_not_pass_a_failed_runtime_with_matching_output() {
        let expected = json!("before");
        let (tool, provider) = fixture(
            SOURCE,
            json!({"status": "failed", "output": expected, "outputs": [expected], "errors": ["assertion failed"]}),
            false,
        );
        let receipt = run(&tool, expected).await;
        assert_eq!(receipt["status"], "failed", "{receipt}");
        assert_eq!(receipt["passed"], false);
        assert_eq!(receipt["runtime"]["errors"], json!(["assertion failed"]));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn draft_tool_reports_ordered_internal_check_identity_across_catalog_drift() {
        let (tool, provider) = fixture(
            SOURCE,
            json!({"status": "success", "output": "before", "outputs": ["before"]}),
            false,
        );
        let original = run(&tool, json!("before")).await;
        assert_eq!(original["status"], "success");

        let mut changed_catalog = provider.catalog.clone();
        changed_catalog[0]
            .description
            .push_str(" Updated contract.");
        *provider.live_catalog.lock().unwrap() = Some(changed_catalog);
        let changed = run(&tool, json!("before")).await;
        assert_eq!(changed["status"], "success", "{changed}");
        assert_eq!(changed["revision"], original["revision"]);
        assert_eq!(
            changed["source_fingerprint"],
            original["source_fingerprint"]
        );
        assert_ne!(
            changed["catalog_fingerprint"],
            original["catalog_fingerprint"]
        );
        assert!(
            changed["validation_sequence"].as_u64().unwrap()
                > original["validation_sequence"].as_u64().unwrap()
        );

        *provider.live_catalog.lock().unwrap() = Some(Vec::new());
        let blocked = run(&tool, json!("before")).await;
        assert_eq!(blocked["status"], "blocked", "{blocked}");
        assert_eq!(blocked["compiler_status"], "validation_errors");
        assert_eq!(blocked["revision"], changed["revision"]);
        assert_eq!(blocked["source_fingerprint"], changed["source_fingerprint"]);
        assert_eq!(blocked["base_fingerprint"], changed["base_fingerprint"]);
        assert_ne!(
            blocked["catalog_fingerprint"],
            changed["catalog_fingerprint"]
        );
        assert!(blocked["commands_fingerprint"].is_null());
        assert!(
            blocked["validation_sequence"].as_u64().unwrap()
                > changed["validation_sequence"].as_u64().unwrap()
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn draft_tool_marks_a_result_stale_when_source_changes_during_execution() {
        let expected = json!("before");
        let (tool, provider) = fixture(
            SOURCE,
            json!({"status": "success", "output": expected, "outputs": [expected]}),
            true,
        );
        let repair = async {
            provider.started.notified().await;
            let patched = tool.store.patch_flowscript_with_acceptance_binding(
                &tool.board,
                &provider.catalog,
                PatchFlowScriptArgs {
                    draft_id: "fixture".into(),
                    expected_revision: 0,
                    old_text: "before".into(),
                    new_text: "after".into(),
                    allow_scope_reduction: false,
                },
                &tool.acceptance_binding,
            );
            assert_eq!(patched.revision, Some(1), "{patched:?}");
            provider.resume.notify_one();
        };
        let (receipt, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(run(&tool, expected), repair)
        })
        .await
        .expect("the test and concurrent repair must finish");
        assert_eq!(receipt["status"], "stale", "{receipt}");
        assert_eq!(receipt["passed"], false);
        assert_eq!(receipt["revision"], 0);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn draft_tool_blocks_invalid_source_before_invoking_runtime() {
        let (tool, provider) = fixture(
            "eventsGeneric testCase() {",
            json!({"status": "success", "output": null, "outputs": [null]}),
            false,
        );
        let receipt = run(&tool, Value::Null).await;
        assert_eq!(receipt["status"], "blocked", "{receipt}");
        assert_eq!(receipt["passed"], false);
        assert!(!receipt["diagnostics"].as_array().unwrap().is_empty());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        assert!(
            tool.store
                .latest_pending_commit_token(&tool.board.id)
                .is_none()
        );
    }
}
