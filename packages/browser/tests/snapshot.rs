use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use flow_like_browser::refs::{NodeRef, RefTable};
use flow_like_browser::snapshot::{AxForest, SnapshotOptions};
use flow_like_browser::testing::{PageHarness, default_auto_reply};
use flow_like_browser::transport::memory::SentCommand;
use flow_like_browser::types::{FrameId, LoaderId, SessionId, TargetId};
use flow_like_browser::{Element, Result};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

const WAIT: Duration = Duration::from_secs(10);

fn stale(reference: &str) -> String {
    format!("Stale element ref '{reference}' — take a new browser snapshot")
}

async fn emit_applied(harness: &PageHarness, session: &str, method: &str, params: Value) {
    let events = harness.connection.events();
    let cursor = events.cursor();
    harness.emit_on(session, method, params);
    let deadline = tokio::time::Instant::now() + WAIT;
    let seen = events
        .wait_for(cursor, deadline, |event| {
            &*event.method == method
                && event.session.as_ref().map(SessionId::as_str) == Some(session)
        })
        .await
        .unwrap();
    assert!(seen.is_some(), "{method} on {session} was never applied");
}

/// T1 (S1) → F1 same-process (S1), F2 cross-process (S2) → F3 nested cross-process (S3).
async fn framed_page() -> PageHarness {
    let harness = PageHarness::new().await;
    emit_applied(
        &harness,
        "S1",
        "Page.frameAttached",
        json!({"frameId": "F1", "parentFrameId": "T1"}),
    )
    .await;
    emit_applied(
        &harness,
        "S1",
        "Page.frameNavigated",
        navigated("F1", Some("T1"), "LF1", "http://127.0.0.1/same"),
    )
    .await;
    harness.context_created("S1", "F1", 3, "S1-F1-util", true);
    harness.attach_child("S2", "F2", "T1", "https://cross.test/b#top", "LF2");
    harness.attach_child("S3", "F3", "F2", "http://127.0.0.1/leaf", "LF3");
    harness
}

fn ax_nodes(frame: &str) -> Value {
    json!([
        {"nodeId": "1", "ignored": false, "role": {"type": "internalRole", "value": "RootWebArea"},
         "name": {"type": "computedString", "value": frame}, "childIds": ["5"], "backendDOMNodeId": 1},
        {"nodeId": "5", "ignored": false, "role": {"type": "role", "value": "button"},
         "name": {"type": "computedString", "value": format!("{frame} button")}, "childIds": [], "backendDOMNodeId": 5},
    ])
}

fn owner_id(session: &str, frame: &str) -> Option<i64> {
    match (session, frame) {
        ("S1", "F1") | ("S2", "F3") => Some(7),
        ("S1", "F2") => Some(8),
        _ => None,
    }
}

fn forest_reply(command: &SentCommand) -> Option<Value> {
    let frame = command.params["frameId"].as_str().unwrap_or_default();
    let session = command.session.as_deref().unwrap_or_default();
    match command.method.as_str() {
        "Accessibility.getFullAXTree" => Some(json!({"nodes": ax_nodes(frame)})),
        "DOM.getFrameOwner" => Some(match owner_id(session, frame) {
            Some(id) => json!({"backendNodeId": id, "nodeId": 0}),
            None => json!({}),
        }),
        "DOMSnapshot.captureSnapshot" => Some(json!({"documents": [], "strings": [session]})),
        _ => default_auto_reply(command),
    }
}

fn withholding(
    method: &str,
    session: &str,
    otherwise: fn(&SentCommand) -> Option<Value>,
) -> impl Fn(&SentCommand) -> Option<Value> + Send + Sync + use<> {
    let (method, session) = (method.to_owned(), session.to_owned());
    move |command| {
        let withheld =
            command.method == method && command.session.as_deref() == Some(session.as_str());
        if withheld { None } else { otherwise(command) }
    }
}

fn navigated(frame: &str, parent: Option<&str>, loader: &str, url: &str) -> Value {
    json!({"frame": {"id": frame, "parentId": parent, "loaderId": loader, "url": url}, "type": "Navigation"})
}

fn node_reply(command: &SentCommand, connected: bool) -> Option<Value> {
    match command.method.as_str() {
        "DOM.resolveNode" => {
            let session = command.session.as_deref().unwrap_or("root");
            let object_id = format!("{session}-{}", command.params["backendNodeId"]);
            Some(json!({"object": {"type": "object", "subtype": "node", "objectId": object_id}}))
        }
        "Runtime.callFunctionOn" => {
            Some(json!({"result": {"type": "boolean", "value": connected}}))
        }
        _ => None,
    }
}

fn connected_nodes(command: &SentCommand) -> Option<Value> {
    node_reply(command, true).or_else(|| default_auto_reply(command))
}

fn node_ref(frame: &str, local_root: &str, loader: &str) -> NodeRef {
    NodeRef {
        page: TargetId::from("T1"),
        local_root: TargetId::from(local_root),
        frame_id: FrameId::from(frame),
        loader_id: LoaderId::from(loader),
        backend_node_id: 5,
    }
}

async fn resolve_error(
    harness: &PageHarness,
    node: &NodeRef,
    reference: &str,
    table_main_loader: Option<&str>,
) -> Option<String> {
    let table_main_loader = table_main_loader.map(LoaderId::from);
    let resolve = harness
        .page
        .resolve_node(node, reference, table_main_loader.as_ref());
    tokio::time::timeout(WAIT, resolve)
        .await
        .expect("the ref op never finished")
        .err()
        .map(|error| error.to_string())
}

fn spawn_resolve(
    harness: &PageHarness,
    node: NodeRef,
    reference: &str,
) -> JoinHandle<Result<Element>> {
    let page = harness.page.clone();
    let reference = reference.to_owned();
    tokio::spawn(async move {
        page.resolve_node(&node, &reference, Some(&LoaderId::from("L1")))
            .await
    })
}

async fn spawned_error(task: JoinHandle<Result<Element>>) -> Option<String> {
    tokio::time::timeout(WAIT, task)
        .await
        .expect("the ref op never finished")
        .unwrap()
        .err()
        .map(|error| error.to_string())
}

fn calls(harness: &PageHarness, method: &str) -> Vec<(String, String)> {
    harness
        .control
        .commands_seen()
        .into_iter()
        .filter(|command| command.method == method)
        .map(|command| {
            let frame = command.params["frameId"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            (command.session.unwrap_or_default(), frame)
        })
        .collect()
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(session, frame)| (session.to_string(), frame.to_string()))
        .collect()
}

fn frame_ids(forest: &AxForest) -> Vec<&str> {
    forest
        .frames
        .iter()
        .map(|frame| frame.frame_id.as_str())
        .collect()
}

async fn forest(harness: &PageHarness, options: SnapshotOptions) -> AxForest {
    tokio::time::timeout(WAIT, harness.page.accessibility_forest(options))
        .await
        .expect("the snapshot never finished")
        .unwrap()
}

fn forest_in_background(harness: &PageHarness) -> JoinHandle<Result<AxForest>> {
    let page = harness.page.clone();
    tokio::spawn(async move { page.accessibility_forest(SnapshotOptions::default()).await })
}

async fn finished(task: JoinHandle<Result<AxForest>>) -> AxForest {
    tokio::time::timeout(WAIT, task)
        .await
        .expect("the snapshot never finished")
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn the_forest_spans_same_process_and_nested_cross_process_frames() {
    let harness = framed_page().await;
    harness.control.set_auto_reply(forest_reply);
    let forest = forest(&harness, SnapshotOptions::default()).await;
    assert_eq!(forest.page, TargetId::from("T1"));
    assert!(forest.warnings.is_empty(), "{:?}", forest.warnings);
    assert_eq!(frame_ids(&forest), ["T1", "F1", "F2", "F3"]);
    let expected = [
        ("L1", "T1", None),
        ("LF1", "T1", Some((0, 7))),
        ("LF2", "F2", Some((0, 8))),
        ("LF3", "F3", Some((2, 7))),
    ];
    for (frame, (loader, local_root, owner)) in forest.frames.iter().zip(expected) {
        let id = frame.frame_id.as_str();
        assert_eq!(frame.loader_id, LoaderId::from(loader), "{id}");
        assert_eq!(frame.local_root, TargetId::from(local_root), "{id}");
        let found = frame
            .owner
            .map(|owner| (owner.parent, owner.backend_node_id));
        assert_eq!(found, owner, "{id}");
        assert_eq!(frame.nodes, ax_nodes(id), "{id}");
    }
    assert_eq!(forest.frames[0].url, "http://127.0.0.1/");
    assert_eq!(forest.frames[0].url_fragment, None);
    assert_eq!(forest.frames[2].url, "https://cross.test/b");
    assert_eq!(forest.frames[2].url_fragment.as_deref(), Some("#top"));
    assert_eq!(forest.dom_snapshots.len(), 3);
    for (root, session) in [("T1", "S1"), ("F2", "S2"), ("F3", "S3")] {
        let snapshot = &forest.dom_snapshots[&TargetId::from(root)];
        assert_eq!(snapshot["strings"], json!([session]), "{root}");
    }
}

#[tokio::test]
async fn forest_commands_run_on_the_owning_and_parent_sessions() {
    let harness = framed_page().await;
    harness.control.set_auto_reply(forest_reply);
    forest(&harness, SnapshotOptions::default()).await;
    assert_eq!(
        calls(&harness, "Accessibility.getFullAXTree"),
        pairs(&[("S1", "T1"), ("S1", "F1"), ("S2", "F2"), ("S3", "F3")])
    );
    assert_eq!(
        calls(&harness, "DOM.getFrameOwner"),
        pairs(&[("S1", "F1"), ("S1", "F2"), ("S2", "F3")]),
        "owners are looked up on the session of the parent frame"
    );
    let every_session = pairs(&[("S1", ""), ("S2", ""), ("S3", "")]);
    assert_eq!(calls(&harness, "Accessibility.enable"), every_session);
    assert_eq!(
        calls(&harness, "DOMSnapshot.captureSnapshot"),
        every_session
    );
    assert_eq!(
        calls(&harness, "Runtime.releaseObjectGroup"),
        pairs(&[]),
        "the snapshot creates no remote objects, so it has no group to release"
    );
}

#[tokio::test]
async fn refs_with_colliding_backend_ids_resolve_in_their_own_frames() {
    let harness = framed_page().await;
    harness
        .control
        .set_auto_reply(|command| node_reply(command, true).or_else(|| forest_reply(command)));
    let forest = forest(&harness, SnapshotOptions::default()).await;
    let main_loader = forest.frames[0].loader_id.clone();
    let mut allocator = RefTable::default().begin(&forest.page, &main_loader);
    for frame in &forest.frames {
        let node = NodeRef {
            page: forest.page.clone(),
            local_root: frame.local_root.clone(),
            frame_id: frame.frame_id.clone(),
            loader_id: frame.loader_id.clone(),
            backend_node_id: 5,
        };
        let proposed = allocator.propose(&node, "button", "Go");
        allocator.commit(proposed, node, "button".into(), "Go".into());
    }
    let table = allocator.finish();
    for (reference, frame, session, context) in [("e2", "F1", "S1", 3), ("e4", "F3", "S3", 2)] {
        let entry = table.lookup(reference).unwrap();
        let element = tokio::time::timeout(
            WAIT,
            harness
                .page
                .resolve_node(&entry.node, reference, table.main_loader()),
        )
        .await
        .expect("the ref never resolved")
        .unwrap();
        assert_eq!(element.reference(), Some(reference));
        assert_eq!(element.frame().id(), &FrameId::from(frame));
        assert_eq!(element.backend_node_id(), 5);
        assert_eq!(element.node_ref(), entry.node);
        let resolved = harness
            .control
            .commands_seen()
            .into_iter()
            .rev()
            .find(|command| command.method == "DOM.resolveNode")
            .unwrap();
        assert_eq!(resolved.session.as_deref(), Some(session), "{reference}");
        assert_eq!(
            resolved.params["executionContextId"], context,
            "{reference}"
        );
        assert_eq!(resolved.params["backendNodeId"], 5, "{reference}");
    }
}

#[tokio::test]
async fn frames_beyond_the_cap_are_left_out_with_a_warning() {
    let harness = framed_page().await;
    harness.control.set_auto_reply(forest_reply);
    let options = SnapshotOptions {
        max_child_frames: 1,
        capture_dom_snapshots: false,
    };
    let forest = forest(&harness, options).await;
    assert_eq!(frame_ids(&forest), ["T1", "F1"]);
    assert_eq!(
        forest.warnings,
        ["The snapshot stops after 1 child frames; 2 more were left out"]
    );
    assert_eq!(
        calls(&harness, "Accessibility.getFullAXTree"),
        pairs(&[("S1", "T1"), ("S1", "F1")])
    );
    assert!(forest.dom_snapshots.is_empty());
    assert!(calls(&harness, "DOMSnapshot.captureSnapshot").is_empty());
    assert!(calls(&harness, "DOMSnapshot.enable").is_empty());
}

#[tokio::test]
async fn a_frame_moving_to_another_process_is_skipped() {
    let harness = framed_page().await;
    harness.control.set_auto_reply(forest_reply);
    emit_applied(
        &harness,
        "S1",
        "Page.frameAttached",
        json!({"frameId": "F2", "parentFrameId": "T1"}),
    )
    .await;
    let forest = forest(&harness, SnapshotOptions::default()).await;
    assert_eq!(frame_ids(&forest), ["T1", "F1"]);
    assert_eq!(
        forest.warnings,
        ["Frame F2 was left out of the snapshot: The frame F2 is moving to another process"]
    );
    assert!(
        !calls(&harness, "Accessibility.getFullAXTree")
            .iter()
            .any(|(_, frame)| frame == "F2")
    );
}

#[tokio::test]
async fn a_frame_chrome_cannot_find_is_skipped_with_its_children() {
    let mut harness = framed_page().await;
    harness.control.set_auto_reply(withholding(
        "Accessibility.getFullAXTree",
        "S2",
        forest_reply,
    ));
    let task = forest_in_background(&harness);
    let read = harness
        .control
        .wait_for("Accessibility.getFullAXTree", Some("S2"))
        .await;
    harness
        .control
        .reply_error(&read, -32602, "Frame with the given frameId is not found.");
    let forest = finished(task).await;
    assert_eq!(frame_ids(&forest), ["T1", "F1"]);
    assert_eq!(
        forest.warnings,
        [
            "Frame F2 was left out of the snapshot: Chrome DevTools command Accessibility.getFullAXTree failed (-32602): Frame with the given frameId is not found.",
            "Frame F3 was left out of the snapshot: its parent frame is not part of the snapshot",
        ]
    );
    assert!(
        !calls(&harness, "DOM.getFrameOwner")
            .iter()
            .any(|(_, frame)| frame == "F2" || frame == "F3")
    );
    assert!(
        !calls(&harness, "Accessibility.getFullAXTree")
            .iter()
            .any(|(_, frame)| frame == "F3")
    );
}

#[tokio::test]
async fn a_frame_without_an_owner_element_is_skipped() {
    let harness = framed_page().await;
    harness.control.set_auto_reply(|command| {
        let orphan = command.method == "DOM.getFrameOwner" && command.params["frameId"] == "F1";
        if orphan {
            Some(json!({}))
        } else {
            forest_reply(command)
        }
    });
    let forest = forest(&harness, SnapshotOptions::default()).await;
    assert_eq!(frame_ids(&forest), ["T1", "F2", "F3"]);
    assert_eq!(
        forest.warnings,
        [
            "Frame F1 was left out of the snapshot: DOM.getFrameOwner named no owner element for frame F1"
        ]
    );
    assert_eq!(forest.frames[2].owner.map(|owner| owner.parent), Some(1));
}

#[tokio::test]
async fn a_failed_dom_snapshot_is_a_warning_without_an_entry() {
    let mut harness = framed_page().await;
    harness.control.set_auto_reply(withholding(
        "DOMSnapshot.captureSnapshot",
        "S2",
        forest_reply,
    ));
    let task = forest_in_background(&harness);
    let capture = harness
        .control
        .wait_for("DOMSnapshot.captureSnapshot", Some("S2"))
        .await;
    harness
        .control
        .reply_error(&capture, -32000, "Document is not available");
    let forest = finished(task).await;
    assert_eq!(frame_ids(&forest), ["T1", "F1", "F2", "F3"]);
    assert_eq!(
        forest.warnings,
        [
            "Browser snapshot skipped clickable-element detection in frame F2: Chrome DevTools command DOMSnapshot.captureSnapshot failed (-32000): Document is not available"
        ]
    );
    let mut roots: Vec<&str> = forest.dom_snapshots.keys().map(TargetId::as_str).collect();
    roots.sort_unstable();
    assert_eq!(roots, ["F3", "T1"]);
}

#[tokio::test]
async fn a_document_change_during_a_dom_snapshot_drops_only_that_snapshot() {
    let mut harness = framed_page().await;
    harness.control.set_auto_reply(withholding(
        "DOMSnapshot.captureSnapshot",
        "S3",
        forest_reply,
    ));
    let task = forest_in_background(&harness);
    let capture = harness
        .control
        .wait_for("DOMSnapshot.captureSnapshot", Some("S3"))
        .await;
    harness.emit_on(
        "S3",
        "Page.frameNavigated",
        navigated("F3", Some("F2"), "LF3b", "http://127.0.0.1/leaf2"),
    );
    harness
        .control
        .reply(&capture, json!({"documents": [], "strings": ["S3"]}));
    let forest = finished(task).await;
    assert_eq!(frame_ids(&forest), ["T1", "F1", "F2", "F3"]);
    assert_eq!(forest.frames[3].loader_id, LoaderId::from("LF3"));
    assert_eq!(
        forest.warnings,
        [
            "Browser snapshot skipped clickable-element detection in frame F3: the document changed while DOMSnapshot.captureSnapshot was running"
        ]
    );
    let mut roots: Vec<&str> = forest.dom_snapshots.keys().map(TargetId::as_str).collect();
    roots.sort_unstable();
    assert_eq!(roots, ["F2", "T1"]);
    assert_eq!(
        calls(&harness, "Accessibility.getFullAXTree").len(),
        4,
        "the snapshot is not restarted"
    );
}

#[tokio::test]
async fn a_parent_document_change_during_an_owner_lookup_skips_the_child() {
    let mut harness = framed_page().await;
    harness
        .control
        .set_auto_reply(withholding("DOM.getFrameOwner", "S2", forest_reply));
    let task = forest_in_background(&harness);
    let lookup = harness
        .control
        .wait_for("DOM.getFrameOwner", Some("S2"))
        .await;
    assert_eq!(lookup.params["frameId"], "F3");
    harness.emit_on(
        "S2",
        "Page.frameNavigated",
        navigated("F2", Some("T1"), "LF2b", "https://cross.test/c"),
    );
    harness
        .control
        .reply(&lookup, json!({"backendNodeId": 7, "nodeId": 0}));
    let forest = finished(task).await;
    assert_eq!(frame_ids(&forest), ["T1", "F1", "F2"]);
    assert_eq!(
        forest.warnings,
        [
            "Frame F3 was left out of the snapshot: the document changed while DOM.getFrameOwner was running",
            "Browser snapshot skipped clickable-element detection in frame F2: the document changed while DOMSnapshot.captureSnapshot was running",
        ]
    );
    assert_eq!(
        calls(&harness, "Accessibility.getFullAXTree").len(),
        4,
        "the snapshot is not restarted"
    );
}

#[tokio::test]
async fn refs_that_do_not_match_the_current_documents_are_stale_before_any_dom_command() {
    let harness = framed_page().await;
    harness.control.set_auto_reply(connected_nodes);
    let live = node_ref("F2", "F2", "LF2");
    let other_page = NodeRef {
        page: TargetId::from("T9"),
        ..live.clone()
    };
    let cases = [
        (
            "an older frame document",
            node_ref("F2", "F2", "LF0"),
            Some("L1"),
        ),
        ("another page", other_page, Some("L1")),
        (
            "another local root",
            node_ref("F2", "T1", "LF2"),
            Some("L1"),
        ),
        (
            "a frame that is gone",
            node_ref("F404", "F404", "LF2"),
            Some("L1"),
        ),
        ("an older main document", live.clone(), Some("L0")),
        ("a table without a main document", live, None),
    ];
    for (case, node, table_main_loader) in cases {
        let error = resolve_error(&harness, &node, "e7", table_main_loader).await;
        assert_eq!(error, Some(stale("e7")), "{case}");
    }
    assert!(
        !harness
            .control
            .commands_seen()
            .iter()
            .any(|command| command.method.starts_with("DOM.")),
        "a stale ref never reaches the renderer"
    );
}

#[tokio::test]
async fn a_ref_goes_stale_when_its_frame_commits_a_new_document() {
    let harness = framed_page().await;
    harness.control.set_auto_reply(connected_nodes);
    let node = node_ref("F2", "F2", "LF2");
    assert_eq!(resolve_error(&harness, &node, "e7", Some("L1")).await, None);
    emit_applied(
        &harness,
        "S2",
        "Page.frameNavigated",
        navigated("F2", Some("T1"), "LF2b", "https://cross.test/c"),
    )
    .await;
    let error = resolve_error(&harness, &node, "e7", Some("L1")).await;
    assert_eq!(error, Some(stale("e7")));
    assert_eq!(calls(&harness, "DOM.resolveNode").len(), 1);
}

#[tokio::test]
async fn a_detached_node_reports_the_stale_ref() {
    let harness = framed_page().await;
    harness.control.set_auto_reply(|command| {
        node_reply(command, false).or_else(|| default_auto_reply(command))
    });
    let error = resolve_error(&harness, &node_ref("F2", "F2", "LF2"), "e3", Some("L1")).await;
    assert_eq!(error, Some(stale("e3")));
}

#[tokio::test]
async fn an_unknown_backend_id_reports_the_stale_ref() {
    let mut harness = framed_page().await;
    harness
        .control
        .set_auto_reply(withholding("DOM.resolveNode", "S2", connected_nodes));
    let task = spawn_resolve(&harness, node_ref("F2", "F2", "LF2"), "e3");
    let resolve = harness
        .control
        .wait_for("DOM.resolveNode", Some("S2"))
        .await;
    harness
        .control
        .reply_error(&resolve, -32000, "No node with given id found");
    assert_eq!(spawned_error(task).await, Some(stale("e3")));
}

#[tokio::test]
async fn a_ref_whose_frame_is_removed_during_resolution_reports_the_stale_ref() {
    let mut harness = framed_page().await;
    harness
        .control
        .set_auto_reply(withholding("DOM.resolveNode", "S1", connected_nodes));
    let task = spawn_resolve(&harness, node_ref("F1", "T1", "LF1"), "e4");
    let resolve = harness
        .control
        .wait_for("DOM.resolveNode", Some("S1"))
        .await;
    let removed = json!({"frameId": "F1", "reason": "remove"});
    harness.emit_on("S1", "Page.frameDetached", removed);
    harness.control.reply(
        &resolve,
        json!({"object": {"type": "object", "subtype": "node", "objectId": "S1-5"}}),
    );
    assert_eq!(spawned_error(task).await, Some(stale("e4")));
}

#[tokio::test]
async fn a_ref_whose_frame_session_detaches_during_resolution_reports_the_stale_ref() {
    let mut harness = framed_page().await;
    harness
        .control
        .set_auto_reply(withholding("DOM.resolveNode", "S3", connected_nodes));
    let task = spawn_resolve(&harness, node_ref("F3", "F3", "LF3"), "e6");
    harness
        .control
        .wait_for("DOM.resolveNode", Some("S3"))
        .await;
    let detached = json!({"sessionId": "S3", "targetId": "F3"});
    harness.emit_on("S2", "Target.detachedFromTarget", detached);
    assert_eq!(spawned_error(task).await, Some(stale("e6")));
}

#[tokio::test]
async fn a_ref_op_on_a_closed_page_is_not_reported_as_stale() {
    let mut harness = framed_page().await;
    harness
        .control
        .set_auto_reply(withholding("DOM.resolveNode", "S3", connected_nodes));
    let task = spawn_resolve(&harness, node_ref("F3", "F3", "LF3"), "e6");
    harness
        .control
        .wait_for("DOM.resolveNode", Some("S3"))
        .await;
    let detached = json!({"sessionId": "S1", "targetId": "T1"});
    harness
        .control
        .emit("Target.detachedFromTarget", None, detached);
    assert_eq!(
        spawned_error(task).await,
        Some("The page or frame closed while DOM.resolveNode was running".to_owned())
    );
}

#[tokio::test]
async fn snapshot_and_resolution_futures_are_send() {
    fn assert_send<T: Send>(_: &T) {}
    let harness = PageHarness::new().await;
    let forest = harness
        .page
        .accessibility_forest(SnapshotOptions::default());
    assert_send(&forest);
    let node = node_ref("T1", "T1", "L1");
    let resolve = harness.page.resolve_node(&node, "e1", None);
    assert_send(&resolve);
}

async fn start_click_navigation(harness: &PageHarness, url: &str) {
    let requested =
        json!({"frameId": "T1", "reason": "anchorClick", "url": url, "disposition": "currentTab"});
    emit_applied(harness, "S1", "Page.frameRequestedNavigation", requested).await;
    let started = json!({"frameId": "T1", "url": url, "loaderId": "L2", "navigationType": "differentDocument"});
    emit_applied(harness, "S1", "Page.frameStartedNavigating", started).await;
}

async fn finish_navigation(harness: &PageHarness, url: &str) {
    let committed = navigated("T1", None, "L2", url);
    emit_applied(harness, "S1", "Page.frameNavigated", committed).await;
    let stopped = json!({"frameId": "T1"});
    emit_applied(harness, "S1", "Page.frameStoppedLoading", stopped).await;
    let loaded = json!({"timestamp": 1.0});
    emit_applied(harness, "S1", "Page.loadEventFired", loaded).await;
}

/// Needs the settling `run_op` of l3-page-settle: the loader comparison runs after its pre-wait.
#[tokio::test]
async fn a_ref_op_after_a_click_started_a_navigation_waits_and_reports_the_stale_ref() {
    let harness = PageHarness::new().await;
    harness.control.set_auto_reply(connected_nodes);
    let next = "http://127.0.0.1/next";
    start_click_navigation(&harness, next).await;
    let resolve = spawn_resolve(&harness, node_ref("T1", "T1", "L1"), "e5");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !resolve.is_finished(),
        "the ref op ran while the navigation of the click was still pending"
    );
    finish_navigation(&harness, next).await;
    assert_eq!(spawned_error(resolve).await, Some(stale("e5")));
    assert!(calls(&harness, "DOM.resolveNode").is_empty());
}

/// Needs the retrying `run_op` of l3-page-settle: a `LoaderChanged` restarts the whole snapshot.
#[tokio::test]
async fn a_document_change_during_the_snapshot_restarts_it() {
    let mut harness = framed_page().await;
    let first_read = Arc::new(AtomicBool::new(true));
    harness.control.set_auto_reply({
        let first_read = first_read.clone();
        move |command| {
            let target = command.method == "Accessibility.getFullAXTree"
                && command.params["frameId"] == "F1";
            if target && first_read.swap(false, Ordering::SeqCst) {
                None
            } else {
                forest_reply(command)
            }
        }
    });
    let task = forest_in_background(&harness);
    let read = harness
        .control
        .wait_for("Accessibility.getFullAXTree", Some("S1"))
        .await;
    assert_eq!(read.params["frameId"], "F1");
    harness.emit_on(
        "S1",
        "Page.frameNavigated",
        navigated("F1", Some("T1"), "LF1b", "http://127.0.0.1/again"),
    );
    harness
        .control
        .reply(&read, json!({"nodes": ax_nodes("F1")}));
    let forest = finished(task).await;
    assert_eq!(frame_ids(&forest), ["T1", "F1", "F2", "F3"]);
    assert_eq!(forest.frames[1].loader_id, LoaderId::from("LF1b"));
    assert_eq!(forest.frames[1].url, "http://127.0.0.1/again");
    let reads = calls(&harness, "Accessibility.getFullAXTree");
    assert_eq!(reads.iter().filter(|(_, frame)| frame == "F1").count(), 2);
}
