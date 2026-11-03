use std::sync::Arc;
use std::sync::atomic::Ordering;

use flow_like_types::bot_message::WorkspaceHandle;
use flow_like_types::data_classification::DataClassification;
use serde_json::{Value, json};

use crate::files::{handle_for, sha256_hex};
use crate::outcome::{cap_value, shrink_string};
use crate::testing::{FakeEnv, HashEmbedder, OTHER_APP, WORKSPACE};
use crate::{Approval, ToolOutcome, run, specs};

fn ok(outcome: &ToolOutcome) -> &ToolOutcome {
    assert!(outcome.is_ok(), "expected ok, got {:?}", outcome.error);
    outcome
}

fn err_code(outcome: &ToolOutcome) -> &'static str {
    assert!(
        !outcome.is_ok(),
        "expected an error, got {:?}",
        outcome.result
    );
    outcome.error.as_ref().expect("error").code
}

async fn call(env: &FakeEnv, tool: &str, args: Value) -> ToolOutcome {
    run(tool, args, env).await
}

mod envelope {
    use super::*;

    #[test]
    fn shrink_keeps_head_and_tail_around_a_marker() {
        let text = "a".repeat(300) + &"b".repeat(300);
        let short = shrink_string(&text, 200);
        assert!(short.len() < 300);
        assert!(short.starts_with("aaa") && short.ends_with("bbb"));
        assert!(short.contains("bytes truncated"));
    }

    #[test]
    fn shrink_never_splits_a_multibyte_char() {
        let text = "é".repeat(500);
        let short = shrink_string(&text, 201);
        assert!(short.contains("truncated"));
        assert!(
            short
                .chars()
                .all(|c| c == 'é' || !c.is_alphabetic() || c.is_ascii())
        );
    }

    #[test]
    fn cap_shortens_the_longest_string_first_and_reports_it() {
        let mut value = json!({ "small": "x", "big": "y".repeat(5000) });
        assert!(cap_value(&mut value, 1000));
        assert!(value.to_string().len() <= 1000);
        assert_eq!(value["small"], "x");
        assert!(value["big"].as_str().expect("string").contains("truncated"));
    }

    #[test]
    fn cap_trims_long_arrays_when_no_string_is_long() {
        let mut value = json!({ "rows": (0..500).collect::<Vec<_>>() });
        assert!(cap_value(&mut value, 300));
        assert!(value.to_string().len() <= 300);
        assert!(!cap_value(&mut json!({ "a": 1 }), 300));
    }

    #[tokio::test]
    async fn run_caps_the_inline_result_and_flags_truncation() {
        let mut env = FakeEnv::new().await;
        env.limits.inline_cap_bytes = 2048;
        for i in 0..150 {
            env.put(
                WORKSPACE,
                &format!("bot/notes/file-number-{i:03}.md"),
                b"x",
                "text/markdown",
            );
        }
        let outcome = call(&env, "file_list", json!({ "prefix": "bot/notes" })).await;
        ok(&outcome);
        assert!(outcome.result.to_string().len() <= 2048);
        assert!(outcome.truncated);
        assert!(!outcome.warnings.is_empty());
        assert!(outcome.result["files"].as_array().expect("files").len() < 150);
    }

    #[tokio::test]
    async fn read_pages_itself_within_the_cap() {
        let mut env = FakeEnv::new().await;
        env.limits.inline_cap_bytes = 4096;
        env.put(
            WORKSPACE,
            "big.txt",
            "z".repeat(50_000).as_bytes(),
            "text/plain",
        );
        let outcome = call(
            &env,
            "file_read",
            json!({ "file": "big.txt", "max_bytes": 1_000_000 }),
        )
        .await;
        ok(&outcome);
        assert!(outcome.envelope().to_string().len() < 6000);
        assert!(outcome.truncated);
        assert!(outcome.result["next_offset"].as_u64().is_some());
    }

    #[tokio::test]
    async fn envelope_has_the_documented_fields() {
        let env = FakeEnv::new().await;
        let outcome = call(
            &env,
            "file_write",
            json!({ "path": "a.txt", "content": "hi" }),
        )
        .await;
        let envelope = outcome.envelope();
        assert_eq!(envelope["status"], "ok");
        assert_eq!(envelope["tool"], "file_write");
        assert_eq!(envelope["saved"].as_array().expect("saved").len(), 1);
        for key in ["truncated", "warnings", "usage", "classification", "handle"] {
            assert!(envelope.get(key).is_some(), "missing {key}");
        }
        assert!(envelope["usage"].get("cpu_ms").is_some());
    }

    #[tokio::test]
    async fn bad_arguments_unknown_tools_and_cancellation_are_errors() {
        let env = FakeEnv::new().await;
        assert_eq!(
            err_code(&call(&env, "file_read", json!({})).await),
            "invalid_args"
        );
        assert_eq!(
            err_code(&call(&env, "nope", json!({})).await),
            "unknown_tool"
        );
        env.cancelled.store(true, Ordering::SeqCst);
        assert_eq!(
            err_code(&call(&env, "file_list", json!({})).await),
            "cancelled"
        );
    }

    #[test]
    fn registry_lists_every_tool_with_an_object_schema_and_dispatches_all() {
        let all = specs();
        let names: Vec<&str> = all.iter().map(|spec| spec.name).collect();
        for expected in [
            "file_list",
            "file_read",
            "file_save",
            "file_write",
            "knowledge_create",
            "knowledge_add",
            "knowledge_search",
            "knowledge_list",
            "knowledge_drop",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
        for spec in &all {
            assert_eq!(spec.input_schema["type"], "object", "{}", spec.name);
            assert!(
                spec.input_schema.get("properties").is_some(),
                "{}",
                spec.name
            );
        }
        let drop = crate::spec("knowledge_drop").expect("spec");
        assert_eq!(drop.approval, Approval::Ask);
        assert_eq!(
            crate::spec("file_write").expect("spec").approval,
            Approval::None
        );
        assert_eq!(drop.capability, Some("knowledge"));
    }

    #[tokio::test]
    async fn every_registered_tool_is_dispatched() {
        let env = FakeEnv::new().await;
        for spec in specs() {
            let outcome = call(&env, spec.name, json!({})).await;
            assert_ne!(
                outcome.error.as_ref().map(|e| e.code),
                Some("unknown_tool"),
                "{} is registered but not dispatched",
                spec.name
            );
        }
    }
}

mod files {
    use super::*;

    #[tokio::test]
    async fn write_then_read_round_trips_with_a_handle() {
        let env = FakeEnv::new().await;
        let written = call(
            &env,
            "file_write",
            json!({ "path": "bot/notes/a.md", "content": "# hello" }),
        )
        .await;
        ok(&written);
        let handle = &written.saved[0];
        assert_eq!(handle.app_id, WORKSPACE);
        assert_eq!(handle.sha256, sha256_hex(b"# hello"));
        assert_eq!(handle.bytes, 7);
        let read = call(&env, "file_read", json!({ "file": "bot/notes/a.md" })).await;
        assert_eq!(ok(&read).result["text"], "# hello");
        assert_eq!(read.result["kind"], "text");
    }

    #[tokio::test]
    async fn write_refuses_anything_but_the_workspace() {
        let env = FakeEnv::new().await.with_storage(OTHER_APP);
        let outcome = call(
            &env,
            "file_write",
            json!({ "path": "x.txt", "content": "no", "app_id": OTHER_APP }),
        )
        .await;
        assert_eq!(err_code(&outcome), "scope_refused");
        assert_eq!(env.write_count(), 0);
        assert!(env.get(OTHER_APP, "x.txt").is_none());
        let own = call(
            &env,
            "file_write",
            json!({ "path": "x.txt", "content": "ok", "app_id": WORKSPACE }),
        )
        .await;
        ok(&own);
        assert_eq!(env.write_count(), 1);
    }

    #[tokio::test]
    async fn write_rejects_unsafe_paths_big_content_and_bad_encodings() {
        let mut env = FakeEnv::new().await;
        env.limits.max_write_bytes = 10;
        for path in ["../x", "a/../b", "a//b", "", "a\\b", "./a"] {
            let outcome = call(&env, "file_write", json!({ "path": path, "content": "x" })).await;
            assert_eq!(err_code(&outcome), "invalid_args", "path {path:?}");
        }
        let big = call(
            &env,
            "file_write",
            json!({ "path": "a", "content": "x".repeat(11) }),
        )
        .await;
        assert_eq!(err_code(&big), "too_large");
        let bad = call(
            &env,
            "file_write",
            json!({ "path": "a", "content": "!!", "encoding": "base64" }),
        )
        .await;
        assert_eq!(err_code(&bad), "invalid_args");
        let good = call(
            &env,
            "file_write",
            json!({ "path": "b.bin", "content": "AAEC", "encoding": "base64" }),
        )
        .await;
        ok(&good);
        assert_eq!(
            env.get(WORKSPACE, "b.bin").expect("file").to_vec(),
            vec![0, 1, 2]
        );
        assert_eq!(env.write_count(), 1);
    }

    #[tokio::test]
    async fn read_pages_long_text_within_the_cap() {
        let mut env = FakeEnv::new().await;
        env.limits.inline_cap_bytes = 2048;
        let text: String = (0..400).map(|i| format!("line {i} ünï\n")).collect();
        env.put(WORKSPACE, "long.txt", text.as_bytes(), "text/plain");
        let mut collected = String::new();
        let mut offset = 0;
        let mut pages = 0;
        loop {
            let outcome = call(
                &env,
                "file_read",
                json!({ "file": "long.txt", "offset": offset }),
            )
            .await;
            ok(&outcome);
            assert!(outcome.envelope().to_string().len() <= 2048 + 512);
            collected.push_str(outcome.result["text"].as_str().expect("text"));
            pages += 1;
            match outcome.result["next_offset"].as_u64() {
                Some(next) => {
                    assert!(outcome.truncated);
                    offset = next as usize;
                }
                None => {
                    assert!(!outcome.truncated);
                    break;
                }
            }
        }
        assert!(pages > 2);
        assert_eq!(collected, text);
    }

    #[tokio::test]
    async fn read_returns_a_handle_for_binary_files() {
        let env = FakeEnv::new().await;
        env.put(
            WORKSPACE,
            "img.png",
            &[0x89, b'P', b'N', b'G', 0, 1, 2],
            "image/png",
        );
        let outcome = call(&env, "file_read", json!({ "file": "img.png" })).await;
        ok(&outcome);
        assert_eq!(outcome.result["kind"], "binary");
        assert_eq!(outcome.result["handle"]["mime"], "image/png");
        assert_eq!(outcome.result["handle"]["bytes"], 7);
        assert!(outcome.result.get("text").is_none());
    }

    #[tokio::test]
    async fn read_converts_documents_and_falls_back_to_a_handle() {
        let env = FakeEnv::new().await;
        env.put(
            WORKSPACE,
            "doc.paged",
            b"page one\x0cpage two",
            "application/x-paged",
        );
        let converted = call(&env, "file_read", json!({ "file": "doc.paged" })).await;
        ok(&converted);
        assert_eq!(converted.result["converted"], true);
        assert!(
            converted.result["text"]
                .as_str()
                .expect("text")
                .contains("page two")
        );
        env.put(WORKSPACE, "doc.pdf", b"%PDF\0\0", "application/pdf");
        let fallback = call(&env, "file_read", json!({ "file": "doc.pdf" })).await;
        ok(&fallback);
        assert_eq!(fallback.result["kind"], "binary");
        assert!(!fallback.warnings.is_empty());
    }

    #[tokio::test]
    async fn read_of_a_missing_or_oversized_file_fails_clearly() {
        let mut env = FakeEnv::new().await;
        assert_eq!(
            err_code(&call(&env, "file_read", json!({ "file": "nope.txt" })).await),
            "not_found"
        );
        env.limits.max_read_bytes = 4;
        env.put(WORKSPACE, "big.txt", b"123456789", "text/plain");
        assert_eq!(
            err_code(&call(&env, "file_read", json!({ "file": "big.txt" })).await),
            "too_large"
        );
    }

    #[tokio::test]
    async fn another_apps_files_need_storage_and_the_app_in_scope() {
        let without_storage = FakeEnv::new().await;
        without_storage.put(OTHER_APP, "d.txt", b"secret", "text/plain");
        let outcome = call(
            &without_storage,
            "file_read",
            json!({ "file": format!("app:{OTHER_APP}/d.txt") }),
        )
        .await;
        assert_eq!(err_code(&outcome), "scope_refused");

        let mut wrong_app = FakeEnv::new().await.with_storage("app-listed");
        wrong_app.put(OTHER_APP, "d.txt", b"secret", "text/plain");
        let outcome = call(
            &wrong_app,
            "file_read",
            json!({ "file": { "app_id": OTHER_APP, "path": "d.txt" } }),
        )
        .await;
        assert_eq!(err_code(&outcome), "scope_refused");
        wrong_app.scope.storage = false;
        wrong_app.put("app-listed", "d.txt", b"x", "text/plain");
        let outcome = call(
            &wrong_app,
            "file_read",
            json!({ "file": "app:app-listed/d.txt" }),
        )
        .await;
        assert_eq!(err_code(&outcome), "scope_refused");

        let allowed = FakeEnv::new().await.with_storage(OTHER_APP);
        allowed.put(OTHER_APP, "d.txt", b"shared", "text/plain");
        let outcome = call(
            &allowed,
            "file_read",
            json!({ "file": format!("app:{OTHER_APP}/d.txt") }),
        )
        .await;
        assert_eq!(ok(&outcome).result["text"], "shared");
    }

    #[tokio::test]
    async fn listing_another_app_is_scope_checked_too() {
        let env = FakeEnv::new().await;
        env.put(OTHER_APP, "d.txt", b"x", "text/plain");
        let outcome = call(&env, "file_list", json!({ "app_id": OTHER_APP })).await;
        assert_eq!(err_code(&outcome), "scope_refused");
    }

    #[tokio::test]
    async fn handles_given_to_the_bot_are_readable_and_checked_by_hash() {
        let env = FakeEnv::new().await;
        env.put(OTHER_APP, "from-mail.txt", b"letter", "text/plain");
        let handle = handle_for(OTHER_APP, "from-mail.txt", b"letter", "text/plain");
        let forged = call(&env, "file_read", json!({ "file": handle })).await;
        assert_eq!(err_code(&forged), "scope_refused");

        env.granted.lock().expect("granted").push(handle.clone());
        let read = call(&env, "file_read", json!({ "file": handle })).await;
        assert_eq!(ok(&read).result["text"], "letter");

        env.put(OTHER_APP, "from-mail.txt", b"edited", "text/plain");
        let stale = call(&env, "file_read", json!({ "file": handle })).await;
        assert_eq!(err_code(&stale), "handle_mismatch");
    }

    #[tokio::test]
    async fn attachments_are_copied_into_the_inbox_on_first_use_only() {
        let env = FakeEnv::new().await;
        env.put(
            "ws-sender",
            "bot/exports/report.txt",
            b"quarterly",
            "text/plain",
        );
        env.attach(
            12,
            "a1",
            "report.txt",
            handle_for(
                "ws-sender",
                "bot/exports/report.txt",
                b"quarterly",
                "text/plain",
            ),
        );
        let first = call(&env, "file_read", json!({ "file": "#12.a1" })).await;
        assert_eq!(ok(&first).result["text"], "quarterly");
        assert_eq!(env.write_count(), 1);
        let copy = env.writes.lock().expect("writes")[0].clone();
        assert_eq!(copy, "bot/inbox/12/a1-report.txt");
        assert_eq!(first.result["handle"]["app_id"], WORKSPACE);
        assert_eq!(first.result["handle"]["path"], copy);

        env.put(
            "ws-sender",
            "bot/exports/report.txt",
            b"changed later",
            "text/plain",
        );
        let second = call(&env, "file_read", json!({ "file": "#12.a1" })).await;
        assert_eq!(ok(&second).result["text"], "quarterly");
        assert_eq!(env.write_count(), 1);
    }

    #[tokio::test]
    async fn attachments_already_in_the_workspace_are_not_copied_and_unknown_ones_fail() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "bot/exports/mine.txt", b"mine", "text/plain");
        env.attach(
            3,
            "a1",
            "mine.txt",
            handle_for(WORKSPACE, "bot/exports/mine.txt", b"mine", "text/plain"),
        );
        ok(&call(&env, "file_read", json!({ "file": "#3.a1" })).await);
        assert_eq!(env.write_count(), 0);
        assert_eq!(
            err_code(&call(&env, "file_read", json!({ "file": "#3.a9" })).await),
            "not_found"
        );
        assert_eq!(
            err_code(&call(&env, "file_read", json!({ "file": "#x.a1" })).await),
            "invalid_args"
        );
    }

    #[tokio::test]
    async fn save_copies_any_source_into_the_workspace() {
        let env = FakeEnv::new().await.with_storage(OTHER_APP);
        env.put(OTHER_APP, "docs/plan.md", b"plan", "text/markdown");
        let saved = call(
            &env,
            "file_save",
            json!({ "file": format!("app:{OTHER_APP}/docs/plan.md") }),
        )
        .await;
        ok(&saved);
        assert_eq!(saved.saved[0].path, "bot/files/plan.md");
        assert_eq!(saved.saved[0].app_id, WORKSPACE);
        assert_eq!(saved.saved[0].mime, "text/markdown");
        assert_eq!(
            env.get(WORKSPACE, "bot/files/plan.md")
                .expect("copy")
                .to_vec(),
            b"plan"
        );

        let named = call(
            &env,
            "file_save",
            json!({ "file": "bot/files/plan.md", "path": "bot/notes/p.md" }),
        )
        .await;
        ok(&named);
        assert_eq!(
            env.get(WORKSPACE, "bot/notes/p.md").expect("copy").to_vec(),
            b"plan"
        );

        let again = call(
            &env,
            "file_save",
            json!({ "file": "bot/notes/p.md", "path": "bot/notes/p.md" }),
        )
        .await;
        ok(&again);
        assert_eq!(env.write_count(), 2);
    }

    #[tokio::test]
    async fn tools_that_save_are_refused_without_a_workspace() {
        let mut env = FakeEnv::new().await;
        env.scope.workspace = None;
        assert_eq!(
            err_code(&call(&env, "file_write", json!({ "path": "a", "content": "b" })).await),
            "no_workspace"
        );
        assert_eq!(
            err_code(&call(&env, "file_save", json!({ "file": "a" })).await),
            "no_workspace"
        );
        assert_eq!(
            err_code(&call(&env, "knowledge_create", json!({ "name": "kb" })).await),
            "no_workspace"
        );
    }

    #[tokio::test]
    async fn list_filters_by_prefix_and_reports_when_cut() {
        let env = FakeEnv::new().await.with_storage(OTHER_APP);
        for name in ["a1", "a2", "a3", "b1"] {
            env.put(
                WORKSPACE,
                &format!("bot/notes/{name}.md"),
                b"x",
                "text/markdown",
            );
        }
        env.put(OTHER_APP, "shared.txt", b"x", "text/plain");
        let all = call(&env, "file_list", json!({ "prefix": "bot/notes/a" })).await;
        assert_eq!(ok(&all).result["files"].as_array().expect("files").len(), 3);
        assert!(!all.truncated);
        let cut = call(
            &env,
            "file_list",
            json!({ "prefix": "bot/notes", "limit": 2 }),
        )
        .await;
        assert_eq!(cut.result["files"].as_array().expect("files").len(), 2);
        assert!(cut.truncated);
        let other = call(&env, "file_list", json!({ "app_id": OTHER_APP })).await;
        assert_eq!(ok(&other).result["files"][0]["path"], "shared.txt");
    }

    #[tokio::test]
    async fn the_files_classification_reaches_the_envelope() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "hr.txt", b"salaries", "text/plain");
        env.classify(WORKSPACE, "hr.txt", DataClassification::Confidential);
        let outcome = call(&env, "file_read", json!({ "file": "hr.txt" })).await;
        assert_eq!(
            ok(&outcome).classification,
            Some(DataClassification::Confidential)
        );
        assert_eq!(outcome.envelope()["classification"], "confidential");
    }
}

mod knowledge {
    use super::*;
    use crate::knowledge::fuse;

    const ANIMALS: &str = "Zebras graze on the savanna in large herds.\n\nPenguins huddle on the ice to keep warm.\n\nDolphins hunt in coordinated pods.";
    const COOKING: &str =
        "Simmer the tomato sauce for an hour.\n\nKnead the pizza dough until it is elastic.";

    async fn stocked() -> FakeEnv {
        let env = FakeEnv::new().await;
        env.put(
            WORKSPACE,
            "bot/files/animals.txt",
            ANIMALS.as_bytes(),
            "text/plain",
        );
        env.put(
            WORKSPACE,
            "bot/files/cooking.txt",
            COOKING.as_bytes(),
            "text/plain",
        );
        ok(&call(
            &env,
            "knowledge_create",
            json!({ "name": "notes", "description": "my notes" }),
        )
        .await);
        env
    }

    #[tokio::test]
    async fn round_trip_finds_the_right_chunk_with_its_source() {
        let env = stocked().await;
        let added = call(&env, "knowledge_add", json!({ "index": "notes", "files": ["bot/files/animals.txt", "bot/files/cooking.txt"] })).await;
        ok(&added);
        assert_eq!(added.result["added"][0]["chunks"], 3);
        assert_eq!(added.result["added"][1]["chunks"], 2);

        let found = call(
            &env,
            "knowledge_search",
            json!({ "query": "penguins on the ice", "k": 2 }),
        )
        .await;
        ok(&found);
        let top = &found.result["hits"][0];
        assert!(
            top["text"]
                .as_str()
                .expect("text")
                .starts_with("Penguins huddle")
        );
        assert_eq!(top["index"], "notes");
        assert_eq!(top["source"]["path"], "bot/files/animals.txt");
        assert_eq!(top["source"]["app_id"], WORKSPACE);
        assert_eq!(top["source"]["sha256"], sha256_hex(ANIMALS.as_bytes()));
        let start = top["start"].as_u64().expect("start") as usize;
        assert_eq!(&ANIMALS[start..start + 8], "Penguins");

        let cooking = call(&env, "knowledge_search", json!({ "query": "pizza dough" })).await;
        assert_eq!(
            cooking.result["hits"][0]["source"]["path"],
            "bot/files/cooking.txt"
        );
    }

    #[tokio::test]
    async fn adding_the_same_source_again_replaces_its_chunks() {
        let env = stocked().await;
        let files = json!({ "index": "notes", "files": ["bot/files/animals.txt"] });
        ok(&call(&env, "knowledge_add", files.clone()).await);
        env.put(
            WORKSPACE,
            "bot/files/animals.txt",
            b"Only the whales remain here.",
            "text/plain",
        );
        let again = call(&env, "knowledge_add", files).await;
        assert_eq!(ok(&again).result["added"][0]["replaced_chunks"], 3);
        assert_eq!(again.result["added"][0]["chunks"], 1);

        let listed = call(&env, "knowledge_list", json!({ "index": "notes" })).await;
        assert_eq!(ok(&listed).result["indexes"][0]["chunks"], 1);
        let old = call(&env, "knowledge_search", json!({ "query": "penguins ice" })).await;
        let texts: Vec<&str> = old.result["hits"]
            .as_array()
            .expect("hits")
            .iter()
            .map(|h| h["text"].as_str().expect("text"))
            .collect();
        assert!(texts.iter().all(|t| !t.contains("Penguins")), "{texts:?}");
        let new = call(&env, "knowledge_search", json!({ "query": "whales" })).await;
        assert!(
            new.result["hits"][0]["text"]
                .as_str()
                .expect("text")
                .contains("whales")
        );
    }

    #[tokio::test]
    async fn drop_removes_the_index_and_its_data() {
        let env = stocked().await;
        ok(&call(
            &env,
            "knowledge_add",
            json!({ "index": "notes", "files": ["bot/files/animals.txt"] }),
        )
        .await);
        let dropped = call(&env, "knowledge_drop", json!({ "index": "notes" })).await;
        assert_eq!(ok(&dropped).result["dropped_chunks"], 3);
        let listed = call(&env, "knowledge_list", json!({})).await;
        assert!(
            ok(&listed).result["indexes"]
                .as_array()
                .expect("indexes")
                .is_empty()
        );
        assert_eq!(
            err_code(
                &call(
                    &env,
                    "knowledge_search",
                    json!({ "query": "zebra", "indexes": ["notes"] })
                )
                .await
            ),
            "not_found"
        );
        assert_eq!(
            err_code(&call(&env, "knowledge_drop", json!({ "index": "notes" })).await),
            "not_found"
        );
        let recreated = call(&env, "knowledge_create", json!({ "name": "notes" })).await;
        assert_eq!(ok(&recreated).result["created"], true);
        assert_eq!(
            call(&env, "knowledge_list", json!({ "index": "notes" }))
                .await
                .result["indexes"][0]["chunks"],
            0
        );
    }

    #[tokio::test]
    async fn another_embedding_model_is_refused_with_advice() {
        let env = stocked().await;
        ok(&call(
            &env,
            "knowledge_add",
            json!({ "index": "notes", "files": ["bot/files/animals.txt"] }),
        )
        .await);
        env.use_embedder(Some(Arc::new(HashEmbedder::new("embed-b", 16))));
        for (tool, args) in [
            (
                "knowledge_add",
                json!({ "index": "notes", "files": ["bot/files/cooking.txt"] }),
            ),
            ("knowledge_search", json!({ "query": "zebra" })),
            ("knowledge_create", json!({ "name": "notes" })),
        ] {
            let outcome = call(&env, tool, args).await;
            assert_eq!(err_code(&outcome), "model_mismatch", "{tool}");
            let message = &outcome.error.as_ref().expect("error").message;
            assert!(message.contains("embed-a") && message.contains("embed-b"));
            assert!(message.contains("knowledge_drop"));
        }
        let created = call(&env, "knowledge_create", json!({ "name": "fresh" })).await;
        assert_eq!(ok(&created).result["embedding_model"], "embed-b");
        let listed = call(&env, "knowledge_list", json!({})).await;
        let models: Vec<&str> = listed.result["indexes"]
            .as_array()
            .expect("indexes")
            .iter()
            .map(|i| i["embedding_model"].as_str().expect("model"))
            .collect();
        assert_eq!(models, ["embed-b", "embed-a"]);
    }

    #[tokio::test]
    async fn a_missing_embedder_is_a_clear_error() {
        let env = FakeEnv::new().await;
        env.use_embedder(None);
        for (tool, args) in [
            ("knowledge_create", json!({ "name": "kb" })),
            ("knowledge_add", json!({ "index": "kb", "files": ["a"] })),
            ("knowledge_search", json!({ "query": "q" })),
        ] {
            let outcome = call(&env, tool, args).await;
            if tool == "knowledge_search" {
                assert_eq!(err_code(&outcome), "not_found");
            } else {
                assert_eq!(err_code(&outcome), "no_embedder", "{tool}");
                assert!(
                    outcome
                        .error
                        .as_ref()
                        .expect("error")
                        .message
                        .contains("embedding model")
                );
            }
        }
        let listed = call(&env, "knowledge_list", json!({})).await;
        ok(&listed);
    }

    #[tokio::test]
    async fn search_without_an_embedder_names_the_missing_model() {
        let env = stocked().await;
        env.use_embedder(None);
        assert_eq!(
            err_code(&call(&env, "knowledge_search", json!({ "query": "zebra" })).await),
            "no_embedder"
        );
    }

    #[tokio::test]
    async fn search_spans_selected_indexes_and_caps_k() {
        let env = stocked().await;
        ok(&call(&env, "knowledge_create", json!({ "name": "recipes" })).await);
        ok(&call(
            &env,
            "knowledge_add",
            json!({ "index": "notes", "files": ["bot/files/animals.txt"] }),
        )
        .await);
        ok(&call(
            &env,
            "knowledge_add",
            json!({ "index": "recipes", "files": ["bot/files/cooking.txt"] }),
        )
        .await);

        let all = call(
            &env,
            "knowledge_search",
            json!({ "query": "pizza dough zebras", "k": 1000 }),
        )
        .await;
        ok(&all);
        let hits = all.result["hits"].as_array().expect("hits");
        assert_eq!(hits.len(), env.limits.knowledge_max_k.min(5));
        let indexes: Vec<&str> = hits
            .iter()
            .map(|h| h["index"].as_str().expect("index"))
            .collect();
        assert!(indexes.contains(&"notes") && indexes.contains(&"recipes"));

        let one = call(
            &env,
            "knowledge_search",
            json!({ "query": "pizza dough zebras", "indexes": ["recipes"], "k": 10 }),
        )
        .await;
        assert!(
            one.result["hits"]
                .as_array()
                .expect("hits")
                .iter()
                .all(|h| h["index"] == "recipes")
        );
        assert_eq!(one.result["hits"].as_array().expect("hits").len(), 2);
    }

    #[tokio::test]
    async fn k_is_capped_by_the_limits() {
        let mut env = FakeEnv::new().await;
        env.limits.knowledge_max_k = 2;
        let text: Vec<String> = (0..6).map(|i| format!("alpha fact number {i}")).collect();
        env.put(
            WORKSPACE,
            "f.txt",
            text.join("\n\n").as_bytes(),
            "text/plain",
        );
        ok(&call(&env, "knowledge_create", json!({ "name": "kb" })).await);
        ok(&call(
            &env,
            "knowledge_add",
            json!({ "index": "kb", "files": ["f.txt"] }),
        )
        .await);
        let found = call(
            &env,
            "knowledge_search",
            json!({ "query": "alpha fact", "k": 50 }),
        )
        .await;
        assert_eq!(ok(&found).result["hits"].as_array().expect("hits").len(), 2);
    }

    #[tokio::test]
    async fn fusion_ranks_chunks_found_by_both_searches_above_either_alone() {
        let embedder = HashEmbedder::new("fixed", 2)
            .fixed("zebra", vec![1.0, 0.0])
            .fixed("zebra stripes", vec![0.0, 1.0])
            .fixed("horse", vec![1.0, 0.0])
            .fixed("zebra horse", vec![0.7, 0.7]);
        let env = FakeEnv::new().await;
        env.use_embedder(Some(Arc::new(embedder)));
        env.put(
            WORKSPACE,
            "f.txt",
            b"zebra stripes\n\nhorse\n\nzebra horse",
            "text/plain",
        );
        ok(&call(&env, "knowledge_create", json!({ "name": "kb" })).await);
        ok(&call(
            &env,
            "knowledge_add",
            json!({ "index": "kb", "files": ["f.txt"] }),
        )
        .await);
        let found = call(
            &env,
            "knowledge_search",
            json!({ "query": "zebra", "k": 3 }),
        )
        .await;
        let order: Vec<&str> = found.result["hits"]
            .as_array()
            .expect("hits")
            .iter()
            .map(|h| h["text"].as_str().expect("text"))
            .collect();
        assert_eq!(order, ["zebra stripes", "zebra horse", "horse"]);
        let scores: Vec<f64> = found.result["hits"]
            .as_array()
            .expect("hits")
            .iter()
            .map(|h| h["score"].as_f64().expect("score"))
            .collect();
        assert!(scores[0] >= scores[1] && scores[1] > scores[2]);
    }

    #[test]
    fn fuse_adds_reciprocal_ranks_across_lists_and_indexes() {
        let row = |id: &str| json!({ "id": id, "text": id });
        let rankings = vec![
            (
                "one".to_string(),
                vec![row("a"), row("b")],
                vec![row("b"), row("c")],
            ),
            ("two".to_string(), vec![row("a")], vec![]),
        ];
        let fused = fuse(&rankings, 10);
        let ids: Vec<(String, String)> = fused
            .iter()
            .map(|r| {
                (
                    r.index.clone(),
                    r.row["id"].as_str().expect("id").to_string(),
                )
            })
            .collect();
        assert_eq!(ids[0], ("one".to_string(), "b".to_string()));
        assert_eq!(ids.len(), 4);
        let b = fused[0].score;
        assert!((b - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-12);
        assert_eq!(fuse(&rankings, 2).len(), 2);
        assert!(fuse(&[], 5).is_empty());
    }

    #[tokio::test]
    async fn partial_failures_are_reported_and_total_failure_is_an_error() {
        let env = stocked().await;
        let mixed = call(&env, "knowledge_add", json!({ "index": "notes", "files": ["bot/files/missing.txt", "bot/files/cooking.txt"] })).await;
        ok(&mixed);
        assert_eq!(mixed.result["added"].as_array().expect("added").len(), 1);
        assert_eq!(mixed.result["failed"][0]["code"], "not_found");
        env.put(
            WORKSPACE,
            "bot/files/x.bin",
            &[0, 1, 2],
            "application/octet-stream",
        );
        env.put(WORKSPACE, "bot/files/blank.txt", b"   \n\n  ", "text/plain");
        let none = call(
            &env,
            "knowledge_add",
            json!({ "index": "notes", "files": ["bot/files/missing.txt", "bot/files/blank.txt"] }),
        )
        .await;
        assert_eq!(err_code(&none), "not_found");
    }

    #[tokio::test]
    async fn adding_respects_scope_for_other_apps() {
        let env = stocked().await;
        env.put(OTHER_APP, "d.txt", b"confidential words", "text/plain");
        let refused = call(
            &env,
            "knowledge_add",
            json!({ "index": "notes", "files": [format!("app:{OTHER_APP}/d.txt")] }),
        )
        .await;
        assert_eq!(err_code(&refused), "scope_refused");
    }

    #[tokio::test]
    async fn attachments_and_converted_documents_index_with_pages() {
        let env = stocked().await;
        env.put(
            "ws-sender",
            "out/book.paged",
            b"Chapter on volcanoes erupting.\x0cChapter on glaciers melting slowly.",
            "application/x-paged",
        );
        env.attach(
            7,
            "a1",
            "book.paged",
            handle_for(
                "ws-sender",
                "out/book.paged",
                b"Chapter on volcanoes erupting.\x0cChapter on glaciers melting slowly.",
                "application/x-paged",
            ),
        );
        let added = call(
            &env,
            "knowledge_add",
            json!({ "index": "notes", "files": ["#7.a1"] }),
        )
        .await;
        ok(&added);
        assert_eq!(
            added.result["added"][0]["source"]["path"],
            "bot/inbox/7/a1-book.paged"
        );
        let found = call(
            &env,
            "knowledge_search",
            json!({ "query": "glaciers melting" }),
        )
        .await;
        assert_eq!(found.result["hits"][0]["page"], 2);
        let first = call(
            &env,
            "knowledge_search",
            json!({ "query": "volcanoes erupting" }),
        )
        .await;
        assert_eq!(first.result["hits"][0]["page"], 1);
    }

    #[tokio::test]
    async fn classification_of_sources_reaches_add_and_search() {
        let env = stocked().await;
        env.classify(
            WORKSPACE,
            "bot/files/cooking.txt",
            DataClassification::Restricted,
        );
        let added = call(
            &env,
            "knowledge_add",
            json!({ "index": "notes", "files": ["bot/files/cooking.txt"] }),
        )
        .await;
        assert_eq!(
            ok(&added).classification,
            Some(DataClassification::Restricted)
        );
        let found = call(&env, "knowledge_search", json!({ "query": "pizza" })).await;
        assert_eq!(
            ok(&found).classification,
            Some(DataClassification::Restricted)
        );
    }

    #[tokio::test]
    async fn list_shows_sources_of_one_index() {
        let env = stocked().await;
        ok(&call(&env, "knowledge_add", json!({ "index": "notes", "files": ["bot/files/animals.txt", "bot/files/cooking.txt"] })).await);
        let listed = call(&env, "knowledge_list", json!({ "index": "notes" })).await;
        let entry = &ok(&listed).result["indexes"][0];
        assert_eq!(entry["description"], "my notes");
        assert_eq!(entry["chunks"], 5);
        assert_eq!(entry["sources"].as_array().expect("sources").len(), 2);
        assert_eq!(entry["dimension"], 16);
        assert_eq!(
            err_code(&call(&env, "knowledge_list", json!({ "index": "ghost" })).await),
            "not_found"
        );
    }

    const DOC_A: &str = "Zebras graze on the savanna.";
    const DOC_B: &str = "Penguins huddle on the ice.";
    const DOC_C: &str = "Dolphins hunt in coordinated pods.";

    fn put_doc(env: &FakeEnv, path: &str, text: &str, modified_ms: i64) {
        env.put(WORKSPACE, path, text.as_bytes(), "text/plain");
        env.touch(WORKSPACE, path, modified_ms);
    }

    async fn sources_of(env: &FakeEnv) -> Vec<String> {
        let listed = call(env, "knowledge_list", json!({ "index": "notes" })).await;
        ok(&listed).result["indexes"][0]["sources"]
            .as_array()
            .expect("sources")
            .iter()
            .map(|source| source["source"].as_str().expect("source").to_string())
            .collect()
    }

    fn doc_folder() -> Value {
        json!({ "index": "notes", "folders": ["bot/docs/"] })
    }

    #[tokio::test]
    async fn a_folder_is_indexed_recursively_within_its_caps() {
        let mut env = stocked().await;
        put_doc(&env, "bot/docs/a.txt", DOC_A, 1);
        put_doc(&env, "bot/docs/b.txt", DOC_B, 1);
        put_doc(&env, "bot/docs/sub/c.txt", DOC_C, 1);
        put_doc(&env, "bot/docs/sub/deep/d.txt", "Otters float.", 1);
        put_doc(&env, "bot/docs2/other.txt", "Not in the folder.", 1);
        env.limits.max_files_per_folder = 3;
        let first = call(&env, "knowledge_add", doc_folder()).await;
        let report = &ok(&first).result["folders"][0];
        assert_eq!(report["listed"], 4);
        assert_eq!(report["added"], 3);
        assert_eq!(report["skipped_total"], 1);
        assert_eq!(report["skipped"][0]["reason"], "per-call cap reached");
        assert_eq!(sources_of(&env).await.len(), 3);

        let second = call(&env, "knowledge_add", doc_folder()).await;
        let report = &ok(&second).result["folders"][0];
        assert_eq!(
            (report["added"].clone(), report["unchanged"].clone()),
            (json!(1), json!(3))
        );
        let sources = sources_of(&env).await;
        assert_eq!(sources.len(), 4);
        assert!(sources.iter().all(|source| source.contains("bot/docs/")));
    }

    #[tokio::test]
    async fn the_folder_byte_cap_skips_what_does_not_fit() {
        let mut env = stocked().await;
        put_doc(&env, "bot/docs/a.txt", DOC_A, 1);
        put_doc(&env, "bot/docs/b.txt", DOC_B, 1);
        env.limits.max_folder_bytes = DOC_A.len() as u64;
        let outcome = call(&env, "knowledge_add", doc_folder()).await;
        let report = &ok(&outcome).result["folders"][0];
        assert_eq!(report["added"], 1);
        assert_eq!(report["skipped_total"], 1);
    }

    #[tokio::test]
    async fn a_folder_of_another_app_needs_storage_access() {
        let env = stocked().await;
        env.put(OTHER_APP, "docs/x.txt", b"Remote text.", "text/plain");
        let args = json!({ "index": "notes", "folders": ["app:app-other/docs/"] });
        assert_eq!(
            err_code(&call(&env, "knowledge_add", args.clone()).await),
            "scope_refused"
        );
        let env = FakeEnv::new().await.with_storage(OTHER_APP);
        env.put(OTHER_APP, "docs/x.txt", b"Remote text.", "text/plain");
        ok(&call(&env, "knowledge_create", json!({ "name": "notes" })).await);
        let allowed = call(&env, "knowledge_add", args).await;
        assert_eq!(ok(&allowed).result["folders"][0]["added"], 1);
        let object =
            json!({ "index": "notes", "folders": [{ "app_id": OTHER_APP, "prefix": "docs" }] });
        let again = call(&env, "knowledge_add", object).await;
        assert_eq!(ok(&again).result["folders"][0]["unchanged"], 1);
    }

    #[tokio::test]
    async fn re_adding_a_folder_indexes_only_new_and_changed_files() {
        let env = stocked().await;
        put_doc(&env, "bot/docs/a.txt", DOC_A, 1);
        put_doc(&env, "bot/docs/b.txt", DOC_B, 1);
        put_doc(&env, "bot/docs/c.txt", DOC_C, 1);
        ok(&call(&env, "knowledge_add", doc_folder()).await);
        env.reads.store(0, std::sync::atomic::Ordering::SeqCst);

        let quiet = call(&env, "knowledge_add", doc_folder()).await;
        let report = &ok(&quiet).result["folders"][0];
        assert_eq!(
            (
                report["added"].clone(),
                report["updated"].clone(),
                report["unchanged"].clone()
            ),
            (json!(0), json!(0), json!(3))
        );
        assert_eq!(env.reads.load(std::sync::atomic::Ordering::SeqCst), 0);

        put_doc(&env, "bot/docs/a.txt", "Zebras sprint on the plains", 2);
        put_doc(&env, "bot/docs/new.txt", "Lemurs leap between branches.", 2);
        let changed = call(&env, "knowledge_add", doc_folder()).await;
        let report = &ok(&changed).result["folders"][0];
        assert_eq!(
            (
                report["added"].clone(),
                report["updated"].clone(),
                report["unchanged"].clone()
            ),
            (json!(1), json!(1), json!(2))
        );
        let found = call(
            &env,
            "knowledge_search",
            json!({ "query": "sprint plains", "k": 1 }),
        )
        .await;
        assert!(
            found.result["hits"][0]["text"]
                .as_str()
                .expect("text")
                .contains("sprint")
        );
        assert_eq!(sources_of(&env).await.len(), 4);
    }

    #[tokio::test]
    async fn a_touched_file_with_the_same_content_is_not_embedded_again() {
        let env = stocked().await;
        put_doc(&env, "bot/docs/a.txt", DOC_A, 1);
        ok(&call(&env, "knowledge_add", doc_folder()).await);
        env.touch(WORKSPACE, "bot/docs/a.txt", 9);
        let outcome = call(&env, "knowledge_add", doc_folder()).await;
        let report = &ok(&outcome).result["folders"][0];
        assert_eq!(
            (report["updated"].clone(), report["unchanged"].clone()),
            (json!(0), json!(1))
        );
        assert_eq!(outcome.usage.model_in, 0);
    }

    #[tokio::test]
    async fn sync_removes_the_chunks_of_vanished_files_only_when_asked() {
        let env = stocked().await;
        put_doc(&env, "bot/docs/a.txt", DOC_A, 1);
        put_doc(&env, "bot/docs/b.txt", DOC_B, 1);
        ok(&call(
            &env,
            "knowledge_add",
            json!({ "index": "notes", "files": ["bot/files/cooking.txt"] }),
        )
        .await);
        ok(&call(&env, "knowledge_add", doc_folder()).await);
        env.files
            .lock()
            .expect("files")
            .remove(&(WORKSPACE.to_string(), "bot/docs/b.txt".to_string()));

        let kept = call(&env, "knowledge_add", doc_folder()).await;
        assert_eq!(ok(&kept).result["folders"][0]["removed"], 0);
        assert_eq!(sources_of(&env).await.len(), 3);

        let mut synced = doc_folder();
        synced["sync"] = json!(true);
        let outcome = call(&env, "knowledge_add", synced).await;
        assert_eq!(ok(&outcome).result["folders"][0]["removed"], 1);
        let sources = sources_of(&env).await;
        assert_eq!(sources.len(), 2);
        assert!(
            sources
                .iter()
                .any(|source| source.ends_with("bot/files/cooking.txt"))
        );
        assert!(sources.iter().all(|source| !source.ends_with("b.txt")));
        let gone = call(
            &env,
            "knowledge_search",
            json!({ "query": "penguins ice huddle" }),
        )
        .await;
        let texts = gone.result["hits"]
            .as_array()
            .expect("hits")
            .iter()
            .map(|hit| hit["text"].as_str().expect("text").to_string())
            .collect::<Vec<_>>();
        assert!(
            texts.iter().all(|text| !text.contains("Penguins")),
            "{texts:?}"
        );
    }

    #[tokio::test]
    async fn refresh_re_syncs_the_remembered_folders() {
        let env = stocked().await;
        assert_eq!(
            err_code(
                &call(
                    &env,
                    "knowledge_add",
                    json!({ "index": "notes", "refresh": true })
                )
                .await
            ),
            "invalid_args"
        );
        put_doc(&env, "bot/docs/a.txt", DOC_A, 1);
        put_doc(&env, "bot/docs/b.txt", DOC_B, 1);
        ok(&call(&env, "knowledge_add", doc_folder()).await);
        let listed = call(&env, "knowledge_list", json!({})).await;
        let folder = &ok(&listed).result["indexes"][0]["folders"][0];
        assert_eq!(
            (
                folder["app_id"].clone(),
                folder["prefix"].clone(),
                folder["files"].clone()
            ),
            (json!(WORKSPACE), json!("bot/docs"), json!(2))
        );
        assert!(folder["last_sync_ms"].as_i64().expect("time") > 0);

        put_doc(&env, "bot/docs/c.txt", DOC_C, 2);
        env.files
            .lock()
            .expect("files")
            .remove(&(WORKSPACE.to_string(), "bot/docs/a.txt".to_string()));
        let refreshed = call(
            &env,
            "knowledge_add",
            json!({ "index": "notes", "refresh": true }),
        )
        .await;
        let report = &ok(&refreshed).result["folders"][0];
        assert_eq!(
            (
                report["added"].clone(),
                report["removed"].clone(),
                report["unchanged"].clone()
            ),
            (json!(1), json!(1), json!(1))
        );
        let sources = sources_of(&env).await;
        assert_eq!(sources.len(), 2);
        assert!(sources.iter().all(|source| !source.ends_with("a.txt")));
    }

    #[tokio::test]
    async fn dropping_an_index_forgets_its_folders() {
        let env = stocked().await;
        put_doc(&env, "bot/docs/a.txt", DOC_A, 1);
        ok(&call(&env, "knowledge_add", doc_folder()).await);
        ok(&call(&env, "knowledge_drop", json!({ "index": "notes" })).await);
        ok(&call(&env, "knowledge_create", json!({ "name": "notes" })).await);
        let listed = call(&env, "knowledge_list", json!({})).await;
        assert!(ok(&listed).result["indexes"][0].get("folders").is_none());
        let again = call(&env, "knowledge_add", doc_folder()).await;
        assert_eq!(ok(&again).result["folders"][0]["added"], 1);
    }

    #[tokio::test]
    async fn names_and_limits_are_validated() {
        let mut env = stocked().await;
        for name in ["", "Has Space", "9lives", "a-b", &"x".repeat(41)] {
            assert_eq!(
                err_code(&call(&env, "knowledge_create", json!({ "name": name })).await),
                "invalid_args",
                "{name:?}"
            );
        }
        let upper = call(&env, "knowledge_create", json!({ "name": "Notes" })).await;
        assert_eq!(ok(&upper).result["created"], false);
        env.limits.max_files_per_add = 1;
        let many = call(
            &env,
            "knowledge_add",
            json!({ "index": "notes", "files": ["a", "b"] }),
        )
        .await;
        assert_eq!(err_code(&many), "invalid_args");
        assert_eq!(
            err_code(
                &call(
                    &env,
                    "knowledge_add",
                    json!({ "index": "notes", "files": [] })
                )
                .await
            ),
            "invalid_args"
        );
        assert_eq!(
            err_code(&call(&env, "knowledge_search", json!({ "query": "  " })).await),
            "invalid_args"
        );
    }

    #[tokio::test]
    async fn read_only_workspace_tables_refuse_changes() {
        let env = stocked().await;
        env.writable.store(false, Ordering::SeqCst);
        assert_eq!(
            err_code(&call(&env, "knowledge_create", json!({ "name": "other" })).await),
            "scope_refused"
        );
        assert_eq!(
            err_code(&call(&env, "knowledge_drop", json!({ "index": "notes" })).await),
            "scope_refused"
        );
        ok(&call(&env, "knowledge_list", json!({})).await);
    }

    #[tokio::test]
    async fn chunk_limit_cuts_with_a_warning() {
        let mut env = FakeEnv::new().await;
        env.limits.max_chunks_per_source = 2;
        env.put(
            WORKSPACE,
            "f.txt",
            b"one\n\ntwo\n\nthree\n\nfour",
            "text/plain",
        );
        ok(&call(&env, "knowledge_create", json!({ "name": "kb" })).await);
        let added = call(
            &env,
            "knowledge_add",
            json!({ "index": "kb", "files": ["f.txt"] }),
        )
        .await;
        assert_eq!(ok(&added).result["added"][0]["chunks"], 2);
        assert!(added.warnings.iter().any(|w| w.contains("cut at 2")));
    }

    #[test]
    fn handles_in_tests_match_the_inbox_shape() {
        let handle: WorkspaceHandle = handle_for("a", "p/q.txt", b"x", "text/plain");
        let value = serde_json::to_value(&handle).expect("json");
        for key in ["app_id", "path", "bytes", "sha256", "mime"] {
            assert!(value.get(key).is_some());
        }
    }
}

mod documents {
    use super::*;
    use crate::Status;

    fn pdf(pages: usize) -> Vec<u8> {
        format!("%PDF-1.7 PAGES:{pages}").into_bytes()
    }

    #[tokio::test]
    async fn summarize_needs_a_goal() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "r.txt", b"report", "text/plain");
        let missing = call(&env, "file_summarize", json!({ "file": "r.txt" })).await;
        assert_eq!(err_code(&missing), "invalid_args");
        for goal in ["", "   \n"] {
            let blank = call(
                &env,
                "file_summarize",
                json!({ "file": "r.txt", "goal": goal }),
            )
            .await;
            assert_eq!(err_code(&blank), "invalid_args", "goal {goal:?}");
        }
        assert_eq!(env.backend().calls(), 0);
        let schema = crate::spec("file_summarize").expect("spec").input_schema;
        assert!(
            schema["required"]
                .as_array()
                .expect("required")
                .contains(&json!("goal"))
        );
    }

    #[tokio::test]
    async fn summarize_passes_the_goal_saves_notes_and_counts_tokens() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "bot/files/q3.pdf", &pdf(12), "application/pdf");
        env.classify(
            WORKSPACE,
            "bot/files/q3.pdf",
            DataClassification::Confidential,
        );
        *env.backend().failed_pages.lock().expect("failed") = vec![7];
        let outcome = call(
            &env,
            "file_summarize",
            json!({ "file": "bot/files/q3.pdf", "goal": " what changed in Q3? " }),
        )
        .await;
        ok(&outcome);
        assert_eq!(
            env.backend().goals.lock().expect("goals")[0],
            "what changed in Q3?"
        );
        assert!(
            outcome.result["summary"]
                .as_str()
                .expect("summary")
                .contains("what changed in Q3?")
        );
        assert_eq!(outcome.result["pages"], 12);
        assert_eq!(outcome.saved[0].path, "bot/notes/q3.summary.md");
        assert_eq!(outcome.result["notes"]["path"], "bot/notes/q3.summary.md");
        assert_eq!(
            (outcome.usage.model_in, outcome.usage.model_out),
            (1200, 80)
        );
        assert_eq!(
            outcome.classification,
            Some(DataClassification::Confidential)
        );
        assert!(outcome.warnings.iter().any(|w| w.contains("[7]")));
    }

    #[tokio::test]
    async fn summarize_without_a_workspace_returns_inline_only() {
        let mut env = FakeEnv::new().await.with_storage(OTHER_APP);
        env.scope.workspace = None;
        env.put(OTHER_APP, "a.txt", b"text", "text/plain");
        let outcome = call(
            &env,
            "file_summarize",
            json!({ "file": format!("app:{OTHER_APP}/a.txt"), "goal": "why" }),
        )
        .await;
        ok(&outcome);
        assert!(outcome.saved.is_empty());
    }

    #[tokio::test]
    async fn big_documents_need_approval_unless_granted() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "huge.pdf", &pdf(150), "application/pdf");
        let asked = call(
            &env,
            "file_summarize",
            json!({ "file": "huge.pdf", "goal": "g" }),
        )
        .await;
        assert_eq!(asked.status, Status::NeedsApproval);
        assert_eq!(asked.envelope()["status"], "needs_approval");
        assert_eq!(asked.error.as_ref().expect("error").code, "needs_approval");
        assert_eq!(env.backend().calls(), 0);

        env.put(
            WORKSPACE,
            "long.txt",
            "word ".repeat(200_000).as_bytes(),
            "text/plain",
        );
        let tokens = call(
            &env,
            "file_summarize",
            json!({ "file": "long.txt", "goal": "g" }),
        )
        .await;
        assert_eq!(tokens.status, Status::NeedsApproval);

        env.expensive_ok.store(true, Ordering::SeqCst);
        ok(&call(
            &env,
            "file_summarize",
            json!({ "file": "huge.pdf", "goal": "g" }),
        )
        .await);
        ok(&call(
            &env,
            "file_summarize",
            json!({ "file": "long.txt", "goal": "g" }),
        )
        .await);
    }

    #[tokio::test]
    async fn a_hundred_pages_is_still_fine() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "ok.pdf", &pdf(100), "application/pdf");
        ok(&call(
            &env,
            "file_summarize",
            json!({ "file": "ok.pdf", "goal": "g" }),
        )
        .await);
    }

    #[tokio::test]
    async fn summarize_respects_scope_and_missing_services() {
        let env = FakeEnv::new().await;
        env.put(OTHER_APP, "a.txt", b"x", "text/plain");
        let refused = call(
            &env,
            "file_summarize",
            json!({ "file": format!("app:{OTHER_APP}/a.txt"), "goal": "g" }),
        )
        .await;
        assert_eq!(err_code(&refused), "scope_refused");
        *env.backend.lock().expect("backend") = None;
        env.put(WORKSPACE, "a.txt", b"x", "text/plain");
        let none = call(
            &env,
            "file_summarize",
            json!({ "file": "a.txt", "goal": "g" }),
        )
        .await;
        assert_eq!(err_code(&none), "unsupported_file");
    }

    #[tokio::test]
    async fn extract_batches_files_and_saves_json() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "inv1.txt", b"ACME invoice 100", "text/plain");
        env.put(WORKSPACE, "inv2.txt", b"Globex invoice 250", "text/plain");
        let outcome = call(&env, "file_extract", json!({
            "files": ["inv1.txt", "inv2.txt"],
            "schema": { "type": "object", "properties": { "first_word": { "type": "string" } } },
            "hint": "vendor only",
        })).await;
        ok(&outcome);
        let results = outcome.result["results"].as_array().expect("results");
        assert_eq!(results.len(), 2);
        assert_eq!(results[1]["value"]["first_word"], "Globex");
        assert_eq!(outcome.saved.len(), 2);
        assert_eq!(outcome.saved[0].path, "bot/extractions/inv1.json");
        assert!(env.get(WORKSPACE, "bot/extractions/inv2.json").is_some());
        assert_eq!(
            env.backend().extractions.lock().expect("e").as_slice(),
            ["vendor only", "vendor only"]
        );
        assert_eq!((outcome.usage.model_in, outcome.usage.model_out), (600, 40));
    }

    #[tokio::test]
    async fn extract_reports_partial_failures_and_total_failure() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "good.txt", b"fine text", "text/plain");
        env.put(WORKSPACE, "bad.txt", b"BROKEN text", "text/plain");
        let schema = json!({ "name": "example" });
        let mixed = call(
            &env,
            "file_extract",
            json!({ "files": ["bad.txt", "good.txt", "gone.txt"], "schema": schema }),
        )
        .await;
        ok(&mixed);
        assert_eq!(
            mixed.result["results"].as_array().expect("results").len(),
            1
        );
        assert_eq!(mixed.result["failed"].as_array().expect("failed").len(), 2);
        let none = call(
            &env,
            "file_extract",
            json!({ "files": ["bad.txt"], "schema": schema }),
        )
        .await;
        assert_eq!(err_code(&none), "failed");
    }

    #[tokio::test]
    async fn extract_validates_its_input() {
        let env = FakeEnv::new().await;
        let bad = call(
            &env,
            "file_extract",
            json!({ "files": ["a.txt"], "schema": "{nope" }),
        )
        .await;
        assert_eq!(err_code(&bad), "invalid_args");
        let empty = call(
            &env,
            "file_extract",
            json!({ "files": [], "schema": {"a": 1} }),
        )
        .await;
        assert_eq!(err_code(&empty), "invalid_args");
        assert_eq!(env.backend().calls(), 0);
    }

    #[tokio::test]
    async fn extract_converts_documents_through_the_converter() {
        let env = FakeEnv::new().await;
        env.put(
            WORKSPACE,
            "d.paged",
            b"alpha page\x0cbeta page",
            "application/x-paged",
        );
        let outcome = call(
            &env,
            "file_extract",
            json!({ "files": ["d.paged"], "schema": {"a": 1} }),
        )
        .await;
        assert_eq!(
            ok(&outcome).result["results"][0]["value"]["first_word"],
            "alpha"
        );
    }

    #[tokio::test]
    async fn pdf_render_saves_every_page_and_shows_at_most_four() {
        let env = FakeEnv::new().await;
        env.put(
            WORKSPACE,
            "bot/files/Deck v2.pdf",
            &pdf(6),
            "application/pdf",
        );
        let outcome = call(
            &env,
            "pdf_render",
            json!({ "file": "bot/files/Deck v2.pdf" }),
        )
        .await;
        ok(&outcome);
        assert_eq!(outcome.saved.len(), 6);
        assert_eq!(outcome.images.len(), 4);
        assert_eq!(outcome.saved[0].path, "bot/renders/Deck v2/page-001.png");
        assert_eq!(outcome.saved[5].mime, "image/png");
        assert!(
            env.get(WORKSPACE, "bot/renders/Deck v2/page-006.png")
                .is_some()
        );
        let pages = outcome.result["pages"].as_array().expect("pages");
        assert_eq!(pages.iter().filter(|p| p["shown"] == true).count(), 4);
        assert_eq!(pages[4]["shown"], false);
        assert!(!outcome.warnings.is_empty());
        assert!(outcome.images[0].label.contains("page 1"));
    }

    #[tokio::test]
    async fn pdf_render_few_pages_show_all_of_them() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "a.pdf", &pdf(10), "application/pdf");
        let outcome = call(
            &env,
            "pdf_render",
            json!({ "file": "a.pdf", "pages": [3, 3, 9] }),
        )
        .await;
        ok(&outcome);
        assert_eq!(env.backend().rendered.lock().expect("r")[0], vec![3, 9]);
        assert_eq!(outcome.images.len(), 2);
        assert!(outcome.warnings.is_empty());
    }

    #[tokio::test]
    async fn pdf_render_rejects_bad_pages_files_and_missing_workspaces() {
        let mut env = FakeEnv::new().await;
        env.put(WORKSPACE, "a.pdf", &pdf(3), "application/pdf");
        env.put(WORKSPACE, "n.txt", b"x", "text/plain");
        for pages in [json!([0]), json!([4]), json!([])] {
            let outcome = call(
                &env,
                "pdf_render",
                json!({ "file": "a.pdf", "pages": pages }),
            )
            .await;
            assert_eq!(err_code(&outcome), "invalid_args");
        }
        assert_eq!(
            err_code(&call(&env, "pdf_render", json!({ "file": "n.txt" })).await),
            "unsupported_file"
        );
        env.scope.workspace = None;
        assert_eq!(
            err_code(&call(&env, "pdf_render", json!({ "file": "a.pdf" })).await),
            "no_workspace"
        );
    }

    #[tokio::test]
    async fn pdf_render_of_more_than_a_hundred_pages_asks_first() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "big.pdf", &pdf(130), "application/pdf");
        let asked = call(&env, "pdf_render", json!({ "file": "big.pdf" })).await;
        assert_eq!(asked.status, Status::NeedsApproval);
        let some = call(
            &env,
            "pdf_render",
            json!({ "file": "big.pdf", "pages": [1, 2] }),
        )
        .await;
        ok(&some);
    }

    #[tokio::test]
    async fn extract_saves_rows_into_a_workspace_table() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "a.txt", b"alpha one", "text/plain");
        env.put(WORKSPACE, "b.txt", b"beta two", "text/plain");
        let outcome = call(
            &env,
            "file_extract",
            json!({ "files": ["a.txt", "b.txt"], "schema": {"a": 1}, "save_to_table": "Invoices" }),
        )
        .await;
        ok(&outcome);
        assert_eq!(outcome.result["saved_table"]["table"], "ws.invoices");
        assert_eq!(outcome.result["saved_table"]["rows"], 2);
        let again = call(
            &env,
            "file_extract",
            json!({ "files": ["a.txt"], "schema": {"a": 1}, "save_to_table": "invoices" }),
        )
        .await;
        assert_eq!(ok(&again).result["saved_table"]["rows"], 1);
        let names = crate::tables::table_names(&env.app_connection(WORKSPACE))
            .await
            .expect("names");
        assert_eq!(names, ["invoices"]);
    }

    #[tokio::test]
    async fn extract_refuses_reserved_or_read_only_tables_before_calling_the_model() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "a.txt", b"alpha", "text/plain");
        for table in ["kb_notes", "__bot_sources__", "kbmeta", "Bad Name"] {
            let outcome = call(
                &env,
                "file_extract",
                json!({ "files": ["a.txt"], "schema": {"a": 1}, "save_to_table": table }),
            )
            .await;
            assert!(!outcome.is_ok(), "{table}");
        }
        assert_eq!(env.backend().calls(), 0);
        env.writable.store(false, Ordering::SeqCst);
        let ro = call(
            &env,
            "file_extract",
            json!({ "files": ["a.txt"], "schema": {"a": 1}, "save_to_table": "t" }),
        )
        .await;
        assert_eq!(err_code(&ro), "scope_refused");
    }

    #[tokio::test]
    async fn excel_inspect_passes_the_request_to_the_backend() {
        let env = FakeEnv::new().await;
        env.put(
            WORKSPACE,
            "book.xlsx",
            b"PK",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        );
        let outcome = call(
            &env,
            "excel_inspect",
            json!({ "file": "book.xlsx", "operation": "range", "sheet": "Q3", "range": "A1:C9" }),
        )
        .await;
        assert_eq!(ok(&outcome).result["text"], "sheet text");
        let styled = call(
            &env,
            "excel_inspect",
            json!({ "file": "book.xlsx", "operation": "find_styled", "style": { "bold": true } }),
        )
        .await;
        ok(&styled);
        let seen = env.backend().excel_requests.lock().expect("excel").clone();
        assert_eq!(
            seen,
            [
                "Range|Some(\"Q3\")|Some(\"A1:C9\")|None",
                "FindStyled|None|None|Some(true)"
            ]
        );
        assert_eq!(
            err_code(
                &call(
                    &env,
                    "excel_inspect",
                    json!({ "file": "book.xlsx", "operation": "nope" })
                )
                .await
            ),
            "invalid_args"
        );
        assert_eq!(
            err_code(
                &call(
                    &env,
                    "excel_inspect",
                    json!({ "file": "gone.xlsx", "operation": "view" })
                )
                .await
            ),
            "not_found"
        );
    }
}

#[cfg(feature = "data")]
mod data {
    use std::time::Duration;

    use flow_like_storage::arrow_schema::{DataType, Field, Schema};
    use flow_like_storage::contracts::column_metadata::{
        CLASSIFICATION_METADATA_KEY, ColumnReader, READ_ATTRIBUTES_KEY,
    };
    use flow_like_storage::databases::vector::VectorStore;
    use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
    use flow_like_storage::lancedb::Connection;

    use super::*;
    use crate::{MemorySessions, SessionCache};

    const CRM: &str = "app-crm-0001";

    async fn seed(connection: &Connection, table: &str, rows: Vec<Value>) {
        LanceDBVectorStore::from_connection(connection.clone(), table.to_string())
            .await
            .insert(rows)
            .await
            .expect("seed");
    }

    async fn stocked() -> FakeEnv {
        let env = FakeEnv::new().await.with_app_tables(CRM, Some("crm")).await;
        seed(&env.app_connection(WORKSPACE), "orders", (1..=3).map(|i| json!({ "id": i, "customer_id": i, "amount": i * 10, "region": if i == 3 { "south" } else { "north" } })).collect()).await;
        seed(
            &env.app_connection(CRM),
            "customers",
            vec![
                json!({ "id": 1, "name": "Acme" }),
                json!({ "id": 2, "name": "Globex" }),
                json!({ "id": 3, "name": "Initech" }),
            ],
        )
        .await;
        env
    }

    #[tokio::test]
    async fn query_joins_workspace_and_app_tables() {
        let env = stocked().await;
        let outcome = call(&env, "data_query", json!({ "sql": "SELECT c.name, o.amount FROM ws.orders o JOIN crm.customers c ON c.id = o.customer_id ORDER BY o.amount" })).await;
        ok(&outcome);
        assert_eq!(outcome.result["row_count"], 3);
        assert_eq!(outcome.result["rows"][0]["name"], "Acme");
        let touched = outcome.result["sources_touched"]
            .as_array()
            .expect("tables");
        assert!(touched.contains(&json!("crm.customers")) && touched.contains(&json!("ws.orders")));
        assert!(outcome.result["elapsed_ms"].is_u64());
        assert_eq!(outcome.result["truncated"], false);
    }

    #[tokio::test]
    async fn query_binds_named_parameters() {
        let env = stocked().await;
        let outcome = call(&env, "data_query", json!({ "sql": "SELECT id FROM ws.orders WHERE region = $r", "params": { "r": "south" } })).await;
        assert_eq!(
            ok(&outcome).result["rows"].as_array().expect("rows").len(),
            1
        );
    }

    #[tokio::test]
    async fn apps_outside_the_scope_cannot_be_read() {
        let env = stocked().await;
        let hidden = "app-secret-9999";
        let described = call(
            &env,
            "data_sources",
            json!({ "operation": "describe", "source": hidden, "table": "salaries" }),
        )
        .await;
        assert_eq!(err_code(&described), "scope_refused");
        let listed = call(
            &env,
            "data_sources",
            json!({ "operation": "describe", "source": hidden }),
        )
        .await;
        assert_eq!(err_code(&listed), "scope_refused");
        let sql = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT * FROM app_secret.salaries" }),
        )
        .await;
        assert!(!sql.is_ok());
        let granted = call(
            &env,
            "data_sources",
            json!({ "operation": "describe", "source": "crm", "table": "customers" }),
        )
        .await;
        ok(&granted);
        let by_id = call(
            &env,
            "data_sources",
            json!({ "operation": "describe", "source": CRM }),
        )
        .await;
        assert_eq!(ok(&by_id).result["tables"], json!(["customers"]));
    }

    #[tokio::test]
    async fn only_one_read_only_statement_runs_and_reserved_tables_stay_closed() {
        let env = stocked().await;
        for sql in [
            "DROP TABLE ws.orders",
            "INSERT INTO ws.orders VALUES (9, 9, 9, 'x')",
            "DELETE FROM ws.orders",
            "SELECT 1; DROP TABLE ws.orders",
            "UPDATE ws.orders SET amount = 0",
        ] {
            let outcome = call(&env, "data_query", json!({ "sql": sql })).await;
            assert_eq!(err_code(&outcome), "scope_refused", "{sql}");
        }
        let blank = call(&env, "data_query", json!({ "sql": "   " })).await;
        assert_eq!(err_code(&blank), "invalid_args");
        let still = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT COUNT(*) AS n FROM ws.orders" }),
        )
        .await;
        assert_eq!(ok(&still).result["rows"][0]["n"], 3);
        for table in ["kb_notes", "kbmeta", "__bot_sources__"] {
            let outcome = call(
                &env,
                "data_query",
                json!({ "sql": "SELECT 1 AS one", "save_as": { "table": table } }),
            )
            .await;
            assert_eq!(err_code(&outcome), "scope_refused", "{table}");
        }
        let bad_file = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT 1 AS one", "save_as": { "file": "../x.csv", "format": "csv" } }),
        )
        .await;
        assert_eq!(err_code(&bad_file), "invalid_args");
        env.writable.store(false, Ordering::SeqCst);
        env.sessions.remove("data.apps");
        let read_only = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT 1 AS one", "save_as": { "table": "t" } }),
        )
        .await;
        assert_eq!(err_code(&read_only), "scope_refused");
    }

    #[tokio::test]
    async fn inline_rows_are_capped() {
        let env = FakeEnv::new().await;
        seed(
            &env.app_connection(WORKSPACE),
            "big",
            (0..300).map(|i| json!({ "id": i })).collect(),
        )
        .await;
        let default = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT id FROM ws.big" }),
        )
        .await;
        ok(&default);
        assert_eq!(default.result["rows"].as_array().expect("rows").len(), 50);
        assert_eq!(default.result["truncated"], true);
        assert!(default.truncated);
        let most = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT id FROM ws.big", "max_rows": 5000 }),
        )
        .await;
        assert_eq!(
            ok(&most).result["rows"].as_array().expect("rows").len(),
            200
        );
        let small = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT id FROM ws.big", "max_rows": 3 }),
        )
        .await;
        assert_eq!(small.result["rows"].as_array().expect("rows").len(), 3);
    }

    #[tokio::test]
    async fn results_can_be_saved_as_tables_and_files() {
        let env = stocked().await;
        let table = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT id, amount FROM ws.orders", "save_as": { "table": "Copy" } }),
        )
        .await;
        ok(&table);
        assert_eq!(table.result["saved"]["table"], "ws.copy");
        assert!(!table.truncated);
        let appended = call(&env, "data_query", json!({ "sql": "SELECT id, amount FROM ws.orders", "save_as": { "table": "copy", "mode": "append" } })).await;
        ok(&appended);
        let count = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT COUNT(*) AS n FROM ws.copy" }),
        )
        .await;
        assert_eq!(count.result["rows"][0]["n"], 6);
        let replaced = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT id, amount FROM ws.orders", "save_as": { "table": "copy" } }),
        )
        .await;
        ok(&replaced);
        let count = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT COUNT(*) AS n FROM ws.copy" }),
        )
        .await;
        assert_eq!(count.result["rows"][0]["n"], 3);

        let csv = call(&env, "data_query", json!({ "sql": "SELECT id, region FROM ws.orders ORDER BY id", "save_as": { "file": "bot/exports/o.csv", "format": "csv" } })).await;
        ok(&csv);
        assert_eq!(csv.saved[0].path, "bot/exports/o.csv");
        let text = String::from_utf8(
            env.get(WORKSPACE, "bot/exports/o.csv")
                .expect("file")
                .to_vec(),
        )
        .expect("utf8");
        assert_eq!(text, "id,region\n1,north\n2,north\n3,south\n");
        let json_file = call(&env, "data_query", json!({ "sql": "SELECT id FROM ws.orders", "save_as": { "file": "bot/exports/o.json", "format": "json" } })).await;
        ok(&json_file);
        assert_eq!(env.get(WORKSPACE, "bot/exports/o.json").is_some(), true);
    }

    async fn salary_table(env: &FakeEnv) {
        let schema = Schema::new(vec![
            Field::new("name", DataType::Utf8, true),
            Field::new("salary", DataType::Int64, true).with_metadata(
                [(
                    CLASSIFICATION_METADATA_KEY.to_string(),
                    "confidential".to_string(),
                )]
                .into(),
            ),
            Field::new("ssn", DataType::Utf8, true)
                .with_metadata([(READ_ATTRIBUTES_KEY.to_string(), "hr".to_string())].into()),
        ]);
        let mut store =
            LanceDBVectorStore::from_connection(env.app_connection(CRM), "staff".to_string()).await;
        store.create_empty_table(schema, true).await.expect("table");
        store
            .insert(vec![json!({ "name": "Ann", "salary": 100, "ssn": "123" })])
            .await
            .expect("rows");
    }

    #[tokio::test]
    async fn classification_is_reported_and_restricted_columns_stay_hidden() {
        let env = stocked().await;
        salary_table(&env).await;
        env.restrict_app(CRM, ColumnReader::with_attributes(Vec::<String>::new()));
        let outcome = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT name, salary FROM crm.staff" }),
        )
        .await;
        ok(&outcome);
        assert_eq!(
            outcome.classification,
            Some(DataClassification::Confidential)
        );
        assert_eq!(outcome.envelope()["classification"], "confidential");
        let hidden = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT ssn FROM crm.staff" }),
        )
        .await;
        assert!(!hidden.is_ok());
        let star = call(&env, "data_sources", json!({ "operation": "describe", "source": "crm", "table": "staff", "include_sample": true })).await;
        ok(&star);
        let names: Vec<&str> = star.result["columns"]
            .as_array()
            .expect("columns")
            .iter()
            .map(|c| c["name"].as_str().expect("name"))
            .collect();
        assert_eq!(names, ["name", "salary"]);
        assert_eq!(star.result["row_count"], 1);
        assert_eq!(star.result["sample"][0]["name"], "Ann");
        assert_eq!(star.result["columns"][1]["classification"], "confidential");
        env.restrict_app(CRM, ColumnReader::Unrestricted);
        env.sessions.remove("data.apps");
        let hr = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT ssn FROM crm.staff" }),
        )
        .await;
        ok(&hr);
    }

    #[tokio::test]
    async fn the_session_cache_keeps_opened_tables_between_calls() {
        let env = stocked().await;
        let sql = json!({ "sql": "SELECT name FROM crm.customers" });
        ok(&call(&env, "data_query", sql.clone()).await);
        let first = env.table_opens.load(Ordering::SeqCst);
        assert_eq!(first, 2);
        ok(&call(&env, "data_query", sql.clone()).await);
        ok(&call(&env, "data_sources", json!({ "operation": "list" })).await);
        assert_eq!(env.table_opens.load(Ordering::SeqCst), first);

        env.use_sessions.store(false, Ordering::SeqCst);
        ok(&call(&env, "data_query", sql).await);
        assert_eq!(env.table_opens.load(Ordering::SeqCst), first + 2);
    }

    #[test]
    fn session_entries_expire() {
        let cache = MemorySessions::default();
        cache.put("a", Arc::new(1_u32), Duration::from_secs(60));
        cache.put("b", Arc::new(2_u32), Duration::ZERO);
        assert_eq!(
            cache
                .get("a")
                .and_then(|v| v.downcast::<u32>().ok())
                .map(|v| *v),
            Some(1)
        );
        assert!(cache.get("b").is_none());
        cache.remove("a");
        assert!(cache.get("a").is_none());
    }

    #[tokio::test]
    async fn list_shows_workspace_tables_apps_and_hides_internal_ones() {
        let env = stocked().await;
        seed(
            &env.app_connection(WORKSPACE),
            "kb_notes",
            vec![json!({ "id": "x" })],
        )
        .await;
        let listed = call(&env, "data_sources", json!({ "operation": "list" })).await;
        ok(&listed);
        assert_eq!(listed.result["workspace"]["tables"], json!(["orders"]));
        assert_eq!(listed.result["apps"][0]["schema"], "crm");
        assert_eq!(listed.result["apps"][0]["tables"], json!(["customers"]));
        assert_eq!(listed.result["attached"], json!([]));
    }

    #[tokio::test]
    async fn csv_files_attach_as_workspace_tables_and_can_be_refreshed_and_detached() {
        let env = stocked().await;
        env.put(
            WORKSPACE,
            "bot/files/sales.csv",
            b"region,total\nnorth,10\nsouth,20\n",
            "text/csv",
        );
        let attach = json!({ "operation": "attach", "attach": { "name": "Sales", "kind": "csv", "file": "bot/files/sales.csv" } });
        let attached = call(&env, "data_sources", attach).await;
        ok(&attached);
        assert_eq!(attached.result["tables"][0]["table"], "ws.ext_sales");
        let rows = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT region FROM ws.ext_sales ORDER BY region" }),
        )
        .await;
        assert_eq!(ok(&rows).result["rows"].as_array().expect("rows").len(), 2);

        let listed = call(&env, "data_sources", json!({ "operation": "list" })).await;
        assert_eq!(listed.result["attached"][0]["name"], "sales");
        assert_eq!(listed.result["attached"][0]["tables"], json!(["ext_sales"]));

        env.put(
            WORKSPACE,
            "bot/files/sales.csv",
            b"region,total\neast,1\n",
            "text/csv",
        );
        ok(&call(
            &env,
            "data_sources",
            json!({ "operation": "refresh", "name": "sales" }),
        )
        .await);
        let after = call(
            &env,
            "data_query",
            json!({ "sql": "SELECT region FROM ws.ext_sales" }),
        )
        .await;
        assert_eq!(after.result["rows"], json!([{ "region": "east" }]));

        let detached = call(
            &env,
            "data_sources",
            json!({ "operation": "detach", "name": "sales" }),
        )
        .await;
        assert_eq!(ok(&detached).result["dropped_tables"], json!(["ext_sales"]));
        assert!(
            !call(
                &env,
                "data_query",
                json!({ "sql": "SELECT * FROM ws.ext_sales" })
            )
            .await
            .is_ok()
        );
        assert_eq!(
            err_code(
                &call(
                    &env,
                    "data_sources",
                    json!({ "operation": "detach", "name": "sales" })
                )
                .await
            ),
            "not_found"
        );
    }

    #[tokio::test]
    async fn workbooks_attach_one_table_per_sheet() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "book.xlsx", b"PK", "application/vnd.ms-excel");
        let attached = call(&env, "data_sources", json!({ "operation": "attach", "attach": { "name": "fin", "kind": "excel", "file": "book.xlsx" } })).await;
        ok(&attached);
        let tables: Vec<&str> = attached.result["tables"]
            .as_array()
            .expect("tables")
            .iter()
            .map(|t| t["name"].as_str().expect("name"))
            .collect();
        assert_eq!(tables, ["ext_fin_sales", "ext_fin_cost_centers"]);
        assert_eq!(attached.warnings, ["merged cells were flattened"]);
        let one = call(&env, "data_sources", json!({ "operation": "attach", "attach": { "name": "fin2", "kind": "excel", "file": "book.xlsx", "excel": { "sheet": "Sales" } } })).await;
        assert_eq!(
            ok(&one).result["tables"].as_array().expect("tables").len(),
            1
        );
    }

    #[tokio::test]
    async fn attaching_files_respects_the_file_scope() {
        let env = stocked().await;
        env.put("app-elsewhere", "data.csv", b"a\n1\n", "text/csv");
        let outcome = call(&env, "data_sources", json!({ "operation": "attach", "attach": { "name": "x", "kind": "csv", "file": "app:app-elsewhere/data.csv" } })).await;
        assert_eq!(err_code(&outcome), "scope_refused");
    }

    fn pg(secret: &str, host: &str) -> Value {
        json!({ "operation": "attach", "attach": {
            "name": "erp", "kind": "postgres",
            "connection": { "secret": secret, "username": "bot", "host": host, "database": "erp", "tables": ["orders"] }
        } })
    }

    #[tokio::test]
    async fn network_databases_need_the_data_connections_capability() {
        let mut env = stocked().await;
        env.set_secret("pg_erp", "hunter2");
        let refused = call(&env, "data_sources", pg("pg_erp", "db.example")).await;
        assert_eq!(err_code(&refused), "scope_refused");
        assert!(
            refused
                .error
                .as_ref()
                .expect("error")
                .message
                .contains("data_connections")
        );
        assert!(env.data().network_calls.lock().expect("calls").is_empty());
        env.scope.data_connections = true;
        let allowed = call(&env, "data_sources", pg("pg_erp", "db.example")).await;
        assert_eq!(
            ok(&allowed).result["tables"][0]["table"],
            "ws.ext_erp_orders"
        );
    }

    #[tokio::test]
    async fn passwords_come_from_secrets_and_never_leave_them() {
        let mut env = stocked().await;
        env.scope.data_connections = true;
        let missing = call(&env, "data_sources", pg("nope", "db.example")).await;
        assert_eq!(err_code(&missing), "not_found");
        assert!(
            missing
                .error
                .as_ref()
                .expect("error")
                .message
                .contains("'nope'")
        );

        env.set_secret("pg_erp", "hunter2");
        let ok_call = call(&env, "data_sources", pg("pg_erp", "db.example")).await;
        ok(&ok_call);
        assert_eq!(
            env.data().network_calls.lock().expect("calls")[0].3,
            "hunter2"
        );
        assert!(!ok_call.envelope().to_string().contains("hunter2"));
        let stored =
            crate::tables::all_json(&env.app_connection(WORKSPACE), crate::tables::SOURCES_TABLE)
                .await
                .expect("rows");
        let stored = stored[0].to_string();
        assert!(stored.contains("pg_erp") && !stored.contains("hunter2"));

        let failed = call(&env, "data_sources", pg("pg_erp", "bad.example")).await;
        assert_eq!(err_code(&failed), "failed");
        let message = &failed.error.as_ref().expect("error").message;
        assert!(
            !message.contains("hunter2") && message.contains("***"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn servers_connect_to_public_addresses_only_and_live_sources_wait() {
        let mut env = stocked().await;
        env.scope.data_connections = true;
        env.set_secret("pg_erp", "pw");
        *env.environment.lock().expect("environment") = Environment::Server;
        ok(&call(&env, "data_sources", pg("pg_erp", "db.example")).await);
        *env.environment.lock().expect("environment") = Environment::Local;
        ok(&call(&env, "data_sources", pg("pg_erp", "db.example")).await);
        let calls = env.data().network_calls.lock().expect("calls").clone();
        assert_eq!((calls[0].1, calls[0].2), (true, true));
        assert_eq!((calls[1].1, calls[1].2), (false, false));
        let live = call(&env, "data_sources", json!({ "operation": "attach", "attach": {
            "name": "l", "kind": "postgres", "mode": "live",
            "connection": { "secret": "pg_erp", "username": "u", "host": "h", "database": "d", "tables": ["t"] } } })).await;
        assert_eq!(err_code(&live), "not_yet");
    }

    #[test]
    fn network_attaches_need_no_approval_but_keep_their_checks() {
        for name in ["data_sources", "data_query"] {
            let spec = crate::spec(name).expect("spec");
            assert!(spec.approval_args.is_empty(), "{name}");
            assert_eq!(spec.approval, crate::Approval::None, "{name}");
        }
    }

    #[test]
    fn data_tools_are_gated_on_data_studio() {
        for name in ["data_sources", "data_query"] {
            assert_eq!(
                crate::spec(name).expect("spec").capability,
                Some("data_studio")
            );
        }
    }

    use crate::Environment;
}

mod code {
    use super::*;
    use crate::{Environment, RunStatus, ToolEnv};

    #[tokio::test]
    async fn runs_stage_inputs_and_save_every_output_as_a_handle() {
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "bot/files/data.csv", b"a,b\n1,2\n", "text/csv");
        env.put(WORKSPACE, "bot/notes/data.csv", b"other", "text/csv");
        let outcome = call(
            &env,
            "code_run",
            json!({
                "language": "Python3",
                "code": "print('hello')",
                "files": ["bot/files/data.csv", "bot/notes/data.csv"],
            }),
        )
        .await;
        ok(&outcome);
        let request = env.sandbox().last();
        assert_eq!(request.language, "python");
        let staged: Vec<&str> = request.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(staged, ["data.csv", "data-2.csv"]);
        assert_eq!(outcome.result["inputs"][1]["path"], "/flow/in/data-2.csv");
        assert!(request.network_hosts.is_empty());
        assert_eq!(request.wall_time.as_secs(), 120);
        assert_eq!(request.memory_bytes, 512 * 1024 * 1024);

        let paths: Vec<&str> = outcome.saved.iter().map(|h| h.path.as_str()).collect();
        let run = outcome.result["run"].as_str().expect("run");
        assert_eq!(
            paths,
            [
                format!("bot/code/{run}/script.py"),
                format!("bot/code/{run}/out/result.csv"),
                format!("bot/code/{run}/in/data.csv"),
            ]
        );
        assert_eq!(
            env.get(WORKSPACE, &paths[1]).expect("file").to_vec(),
            b"a,b\n1,2\n"
        );
        assert_eq!(outcome.result["stdout"], "hello\n");
        assert_eq!(outcome.result["outputs"]["x"], 1);
        assert_eq!(outcome.result["files"].as_array().expect("files").len(), 3);
    }

    #[tokio::test]
    async fn the_network_is_off_unless_hosts_are_named() {
        let env = FakeEnv::new().await;
        ok(&call(
            &env,
            "code_run",
            json!({ "language": "python", "code": "pass" }),
        )
        .await);
        assert!(env.sandbox().last().network_hosts.is_empty());
        ok(&call(&env, "code_run", json!({ "language": "python", "code": "pass", "network_hosts": ["API.Example.com", "api.example.com"] })).await);
        assert_eq!(env.sandbox().last().network_hosts, ["api.example.com"]);
        let spec = crate::spec("code_run").expect("spec");
        assert_eq!(spec.approval_args, ["network_hosts"]);
        assert_eq!(spec.capability, Some("code_execution"));
    }

    #[tokio::test]
    async fn hosts_are_checked_for_syntax_and_count() {
        let mut env = FakeEnv::new().await;
        for host in ["https://x.com", "a b", "x.com/path", ""] {
            let outcome = call(
                &env,
                "code_run",
                json!({ "language": "python", "code": "pass", "network_hosts": [host] }),
            )
            .await;
            assert_eq!(err_code(&outcome), "invalid_args", "{host:?}");
        }
        env.limits.code_max_hosts = 1;
        let many = call(
            &env,
            "code_run",
            json!({ "language": "python", "code": "pass", "network_hosts": ["a.com", "b.com"] }),
        )
        .await;
        assert_eq!(err_code(&many), "invalid_args");
        assert!(env.sandbox().requests.lock().expect("requests").is_empty());
    }

    #[tokio::test]
    async fn servers_refuse_hosts_that_are_not_public() {
        let env = FakeEnv::new().await;
        *env.environment.lock().expect("environment") = Environment::Server;
        for host in [
            "127.0.0.1",
            "10.0.0.5",
            "169.254.169.254",
            "localhost",
            "db.internal",
            "[::1]",
        ] {
            let outcome = call(
                &env,
                "code_run",
                json!({ "language": "python", "code": "pass", "network_hosts": [host] }),
            )
            .await;
            assert_eq!(err_code(&outcome), "scope_refused", "{host}");
        }
        assert!(env.sandbox().requests.lock().expect("requests").is_empty());
        ok(&call(
            &env,
            "code_run",
            json!({ "language": "python", "code": "pass", "network_hosts": ["93.184.216.34"] }),
        )
        .await);
        *env.environment.lock().expect("environment") = Environment::Local;
        ok(&call(
            &env,
            "code_run",
            json!({ "language": "python", "code": "pass", "network_hosts": ["10.0.0.5"] }),
        )
        .await);
    }

    #[tokio::test]
    async fn input_files_follow_the_file_scope_rules() {
        let env = FakeEnv::new().await;
        env.put(OTHER_APP, "d.csv", b"x", "text/csv");
        let refused = call(&env, "code_run", json!({ "language": "python", "code": "pass", "files": [format!("app:{OTHER_APP}/d.csv")] })).await;
        assert_eq!(err_code(&refused), "scope_refused");
        assert!(env.sandbox().requests.lock().expect("requests").is_empty());
        let missing = call(
            &env,
            "code_run",
            json!({ "language": "python", "code": "pass", "files": ["nope.csv"] }),
        )
        .await;
        assert_eq!(err_code(&missing), "not_found");

        let allowed = FakeEnv::new().await.with_storage(OTHER_APP);
        allowed.put(OTHER_APP, "d.csv", b"x", "text/csv");
        ok(&call(&allowed, "code_run", json!({ "language": "python", "code": "pass", "files": [format!("app:{OTHER_APP}/d.csv")] })).await);
        assert_eq!(allowed.sandbox().last().files[0].0, "d.csv");

        let mail = FakeEnv::new().await;
        mail.put("ws-sender", "out/r.txt", b"mail", "text/plain");
        mail.attach(
            4,
            "a1",
            "r.txt",
            crate::files::handle_for("ws-sender", "out/r.txt", b"mail", "text/plain"),
        );
        ok(&call(
            &mail,
            "code_run",
            json!({ "language": "python", "code": "pass", "files": ["#4.a1"] }),
        )
        .await);
        assert_eq!(mail.sandbox().last().files[0].0, "a1-r.txt");
    }

    #[tokio::test]
    async fn runtime_statuses_become_clear_errors_that_keep_the_output() {
        let env = FakeEnv::new().await;
        for (status, expected) in [
            (RunStatus::Error, "code_error"),
            (RunStatus::Timeout, "code_timeout"),
            (RunStatus::OutOfMemory, "code_out_of_memory"),
            (RunStatus::FuelExhausted, "code_fuel_exhausted"),
            (RunStatus::Unavailable, "code_unavailable"),
        ] {
            env.sandbox()
                .respond(status, "partial output", Some("boom"));
            let outcome = call(
                &env,
                "code_run",
                json!({ "language": "python", "code": "x" }),
            )
            .await;
            assert_eq!(err_code(&outcome), expected);
            let envelope = outcome.envelope();
            assert_eq!(envelope["stdout"], "partial output");
            assert_eq!(envelope["stderr"], "trace");
            assert!(
                envelope["error"]["message"]
                    .as_str()
                    .expect("message")
                    .contains("boom")
            );
        }
        assert!(env.get(WORKSPACE, "bot/code").is_none());
    }

    #[tokio::test]
    async fn long_output_keeps_head_and_tail() {
        let env = FakeEnv::new().await;
        let long = format!("START{}END", "x".repeat(60_000));
        env.sandbox().respond(RunStatus::Ok, &long, None);
        let outcome = call(
            &env,
            "code_run",
            json!({ "language": "python", "code": "x" }),
        )
        .await;
        ok(&outcome);
        let stdout = outcome.result["stdout"].as_str().expect("stdout");
        assert!(
            stdout.starts_with("START") && stdout.ends_with("END") && stdout.contains("truncated")
        );
        assert!(outcome.truncated);
        assert!(outcome.envelope().to_string().len() <= env.limits.inline_cap_bytes);
    }

    #[tokio::test]
    async fn languages_and_missing_sandboxes_are_reported() {
        let env = FakeEnv::new().await;
        assert_eq!(env.code_languages(), ["python"]);
        let ruby = call(
            &env,
            "code_run",
            json!({ "language": "ruby", "code": "puts 1" }),
        )
        .await;
        assert_eq!(err_code(&ruby), "code_unavailable");
        assert!(
            ruby.error
                .as_ref()
                .expect("error")
                .message
                .contains("python")
        );
        assert_eq!(
            err_code(
                &call(
                    &env,
                    "code_run",
                    json!({ "language": "python", "code": "  " })
                )
                .await
            ),
            "invalid_args"
        );
        *env.sandbox.lock().expect("sandbox") = None;
        assert!(env.code_languages().is_empty());
        assert_eq!(
            err_code(
                &call(
                    &env,
                    "code_run",
                    json!({ "language": "python", "code": "x" })
                )
                .await
            ),
            "code_unavailable"
        );
    }

    fn run_with_files(files: Vec<(&str, &[u8])>) -> crate::RunResponse {
        crate::RunResponse {
            status: RunStatus::Ok,
            outputs: json!({}),
            stdout: String::new(),
            stderr: String::new(),
            error: None,
            files: files
                .into_iter()
                .map(|(path, bytes)| {
                    (
                        path.to_string(),
                        flow_like_types::Bytes::copy_from_slice(bytes),
                    )
                })
                .collect(),
            truncated: false,
            duration_ms: 1,
        }
    }

    #[tokio::test]
    async fn images_are_shown_and_every_file_is_a_handle() {
        let env = FakeEnv::new().await;
        *env.sandbox().reply.lock().expect("reply") = Some(run_with_files(vec![
            ("out/plot.png", b"\x89PNG-bytes"),
            ("out/photo.JPG", b"jpg-bytes"),
            ("out/figure.svg", b"<svg/>"),
            ("out/table.csv", b"a,b\n1,2\n"),
            ("out/data.json", b"{\"a\":1}"),
        ]));
        let outcome = call(
            &env,
            "code_run",
            json!({ "language": "python", "code": "x" }),
        )
        .await;
        ok(&outcome);
        let shown: Vec<(&str, &str)> = outcome
            .images
            .iter()
            .map(|i| (i.label.as_str(), i.mime.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("out/plot.png", "image/png"),
                ("out/photo.JPG", "image/jpeg")
            ]
        );
        let mimes: Vec<&str> = outcome.saved.iter().map(|h| h.mime.as_str()).collect();
        for expected in ["image/png", "image/svg+xml", "text/csv", "application/json"] {
            assert!(mimes.contains(&expected), "{expected} in {mimes:?}");
        }
        assert!(
            outcome
                .saved
                .iter()
                .any(|h| h.path.ends_with("out/table.csv"))
        );
        assert!(outcome.warnings.iter().any(|w| w.contains("SVG")));
    }

    #[tokio::test]
    async fn images_are_capped_in_count_and_size() {
        let env = FakeEnv::new().await;
        let big = vec![0u8; 5 * 1024 * 1024];
        let mut files: Vec<(String, Vec<u8>)> = (0..6)
            .map(|n| (format!("out/p{n}.png"), vec![1u8; 10]))
            .collect();
        files.push(("out/big.png".to_string(), big));
        *env.sandbox().reply.lock().expect("reply") = Some(run_with_files(
            files
                .iter()
                .map(|(p, b)| (p.as_str(), b.as_slice()))
                .collect(),
        ));
        let outcome = call(
            &env,
            "code_run",
            json!({ "language": "python", "code": "x" }),
        )
        .await;
        ok(&outcome);
        assert_eq!(outcome.images.len(), 4);
        assert!(
            outcome
                .warnings
                .iter()
                .any(|w| w.contains("3 image(s) were not shown"))
        );
        assert_eq!(outcome.saved.len(), 1 + 7);
    }

    #[tokio::test]
    async fn javascript_aliases_and_packages_reach_the_sandbox() {
        let env = FakeEnv::new().await;
        let outcome = call(
            &env,
            "code_run",
            json!({ "language": "python", "code": "x", "packages": ["NumPy", "pandas", "numpy"] }),
        )
        .await;
        ok(&outcome);
        assert_eq!(env.sandbox().last().packages, ["numpy", "pandas"]);

        let refused = call(
            &env,
            "code_run",
            json!({ "language": "py", "code": "x", "packages": [" "] }),
        )
        .await;
        assert_eq!(err_code(&refused), "invalid_args");
    }

    #[test]
    fn language_aliases_normalise() {
        for alias in ["js", "Node", "nodejs", "QuickJS"] {
            assert_eq!(crate::code::language_id_for_tests(alias), "javascript");
        }
        assert_eq!(crate::code::language_id_for_tests("PY"), "python");
    }

    #[tokio::test]
    async fn without_a_workspace_nothing_is_saved() {
        let mut env = FakeEnv::new().await;
        env.scope.workspace = None;
        let outcome = call(
            &env,
            "code_run",
            json!({ "language": "python", "code": "x" }),
        )
        .await;
        ok(&outcome);
        assert!(outcome.saved.is_empty());
        assert!(outcome.warnings.iter().any(|w| w.contains("no workspace")));
        assert_eq!(env.write_count(), 0);
    }
}

#[cfg(feature = "code")]
mod real_sandbox {
    use super::*;

    #[tokio::test]
    #[ignore = "downloads the Python runtime; run with SANDBOX_TEST_DOWNLOAD=1 -- --ignored"]
    async fn python_runs_for_real() {
        if std::env::var("SANDBOX_TEST_DOWNLOAD").as_deref() != Ok("1") {
            return;
        }
        let env = FakeEnv::new().await;
        env.put(WORKSPACE, "a.txt", b"hello", "text/plain");
        *env.real_sandbox.lock().expect("real") =
            Some(Arc::new(crate::code::CatalogSandbox::shared()));
        let outcome = call(&env, "code_run", json!({
            "language": "python",
            "code": "print(open('/flow/in/a.txt').read().upper())\nopen('b.txt','w').write('done')",
            "files": ["a.txt"],
        })).await;
        ok(&outcome);
        assert!(
            outcome.result["stdout"]
                .as_str()
                .expect("stdout")
                .contains("HELLO")
        );
        assert!(outcome.saved.iter().any(|h| h.path.ends_with("out/b.txt")));
    }

    async fn real_env() -> Option<FakeEnv> {
        if std::env::var("SANDBOX_TEST_DOWNLOAD").as_deref() != Ok("1") {
            return None;
        }
        let env = FakeEnv::new().await;
        *env.real_sandbox.lock().expect("real") =
            Some(Arc::new(crate::code::CatalogSandbox::shared()));
        Some(env)
    }

    #[tokio::test]
    #[ignore = "downloads the QuickJS runtime; run with SANDBOX_TEST_DOWNLOAD=1 -- --ignored"]
    async fn javascript_runs_for_real() {
        let Some(env) = real_env().await else { return };
        env.put(WORKSPACE, "a.csv", b"x,y\n1,2\n", "text/csv");
        let outcome = call(&env, "code_run", json!({
            "language": "js",
            "code": "import * as std from 'std';\nconst t = std.loadFile('/flow/in/a.csv');\nconsole.log(t.split('\\n').length);\noutputs.ok = true;\nconst f = std.open('o.json', 'w'); f.puts(JSON.stringify({n: 1})); f.close();",
            "files": ["a.csv"],
        })).await;
        ok(&outcome);
        assert_eq!(outcome.result["language"], "javascript");
        assert_eq!(outcome.result["outputs"]["ok"], true);
        assert!(
            outcome
                .saved
                .iter()
                .any(|h| h.path.ends_with("out/o.json") && h.mime == "application/json")
        );
        assert!(outcome.saved.iter().any(|h| h.path.ends_with("script.js")));
    }

    #[tokio::test]
    #[ignore = "downloads the Python runtime; run with SANDBOX_TEST_DOWNLOAD=1 -- --ignored"]
    async fn a_png_written_by_python_comes_back_as_an_image_and_a_handle() {
        let Some(env) = real_env().await else { return };
        let outcome = call(&env, "code_run", json!({
            "language": "python",
            "code": "import zlib, struct\n\ndef chunk(kind, data):\n    body = kind + data\n    return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body))\n\nraw = b'\\x00\\xff\\x00\\x00'\npng = b'\\x89PNG\\r\\n\\x1a\\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')\nopen('dot.png', 'wb').write(png)\nopen('t.csv', 'w').write('a\\n1\\n')",
        })).await;
        ok(&outcome);
        assert_eq!(outcome.images.len(), 1);
        assert_eq!(outcome.images[0].mime, "image/png");
        assert_eq!(&outcome.images[0].data[1..4], b"PNG");
        assert!(
            outcome
                .saved
                .iter()
                .any(|h| h.path.ends_with("out/dot.png") && h.mime == "image/png")
        );
        assert!(
            outcome
                .saved
                .iter()
                .any(|h| h.path.ends_with("out/t.csv") && h.mime == "text/csv")
        );
    }

    #[tokio::test]
    #[ignore = "downloads ~98 MB; run with SANDBOX_TEST_DOWNLOAD=1 SANDBOX_TEST_SCIENTIFIC=1 -- --ignored"]
    async fn packages_load_the_scientific_build() {
        if std::env::var("SANDBOX_TEST_SCIENTIFIC").as_deref() != Ok("1") {
            return;
        }
        let Some(env) = real_env().await else { return };
        let outcome = call(&env, "code_run", json!({
            "language": "python",
            "packages": ["numpy", "pandas"],
            "code": "import numpy as np, pandas as pd\noutputs['s'] = float(np.arange(4).sum())\npd.DataFrame({'a': [1, 2]}).to_csv('d.csv', index=False)",
        })).await;
        ok(&outcome);
        assert_eq!(outcome.result["outputs"]["s"], 6.0);
        assert!(outcome.saved.iter().any(|h| h.path.ends_with("out/d.csv")));
    }
}
