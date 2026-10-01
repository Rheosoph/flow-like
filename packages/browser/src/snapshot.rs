use std::collections::{HashMap, HashSet};
use std::fmt::Display;

use serde_json::{Value, json};

use crate::error::{BrowserError, ErrorClass};
use crate::page::{Frame, FrameStamp, Page};
use crate::settle::OpSpec;
use crate::types::{FrameId, LoaderId, SessionId, TargetId};

const LOG_TARGET: &str = "flow_like_browser::snapshot";

#[derive(Clone, Copy, Debug)]
pub struct SnapshotOptions {
    pub max_child_frames: usize,
    pub capture_dom_snapshots: bool,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            max_child_frames: 20,
            capture_dom_snapshots: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AxFrame {
    pub frame_id: FrameId,
    pub loader_id: LoaderId,
    pub local_root: TargetId,
    pub url: String,
    pub url_fragment: Option<String>,
    pub owner: Option<FrameOwner>,
    pub nodes: serde_json::Value,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameOwner {
    pub parent: usize,
    pub backend_node_id: i64,
}

#[derive(Clone, Debug)]
pub struct AxForest {
    pub page: TargetId,
    pub frames: Vec<AxFrame>,
    pub dom_snapshots: std::collections::HashMap<TargetId, serde_json::Value>,
    pub warnings: Vec<String>,
}

impl Page {
    pub async fn accessibility_forest(&self, options: SnapshotOptions) -> crate::Result<AxForest> {
        let main = self.main_frame();
        self.run_op(main.id(), OpSpec::READ, |_| capture_forest(self, options))
            .await?
            .read()
    }
}

struct FrameRead {
    nodes: Value,
    url: String,
    url_fragment: Option<String>,
}

struct ForestBuilder {
    forest: AxForest,
    stamps: Vec<FrameStamp>,
    positions: HashMap<FrameId, usize>,
    accessibility_enabled: HashSet<SessionId>,
}

impl ForestBuilder {
    fn new(page: TargetId) -> Self {
        Self {
            forest: AxForest {
                page,
                frames: Vec::new(),
                dom_snapshots: HashMap::new(),
                warnings: Vec::new(),
            },
            stamps: Vec::new(),
            positions: HashMap::new(),
            accessibility_enabled: HashSet::new(),
        }
    }

    fn push(
        &mut self,
        frame: &FrameId,
        stamp: FrameStamp,
        read: FrameRead,
        owner: Option<FrameOwner>,
    ) {
        self.positions
            .insert(frame.clone(), self.forest.frames.len());
        self.forest.frames.push(AxFrame {
            frame_id: frame.clone(),
            loader_id: stamp.loader.clone(),
            local_root: stamp.local_root.clone(),
            url: read.url,
            url_fragment: read.url_fragment,
            owner,
            nodes: read.nodes,
        });
        self.stamps.push(stamp);
    }

    fn skip(&mut self, frame: &FrameId, reason: impl Display) {
        self.forest.warnings.push(format!(
            "Frame {frame} was left out of the snapshot: {reason}"
        ));
    }

    fn main_unchanged(&self, page: &Page, method: &str) -> crate::Result<()> {
        if stamp_unchanged(page, &self.forest.frames[0].frame_id, &self.stamps[0]) {
            Ok(())
        } else {
            Err(loader_changed(method))
        }
    }

    fn local_roots(&self) -> Vec<(FrameId, TargetId, FrameStamp)> {
        let mut seen = HashSet::new();
        self.forest
            .frames
            .iter()
            .zip(&self.stamps)
            .filter(|(frame, _)| seen.insert(frame.local_root.clone()))
            .map(|(frame, stamp)| {
                (
                    frame.frame_id.clone(),
                    frame.local_root.clone(),
                    stamp.clone(),
                )
            })
            .collect()
    }
}

fn frame_of(page: &Page, id: FrameId) -> Frame {
    Frame {
        page: page.clone(),
        id,
    }
}

fn document_url(page: &Page, frame: &FrameId) -> (String, Option<String>) {
    page.inner
        .lock_state()
        .frames
        .get(frame)
        .map(|node| (node.url.clone(), node.url_fragment.clone()))
        .unwrap_or_default()
}

fn forest_frames(page: &Page, max_child_frames: usize) -> (FrameId, Vec<FrameId>, usize) {
    let state = page.inner.lock_state();
    let main = state.frames.main_id().clone();
    let mut children = state.frames.descendants_preorder(&main);
    let omitted = children.len().saturating_sub(max_child_frames);
    children.truncate(max_child_frames);
    (main, children, omitted)
}

fn parent_position(page: &Page, builder: &ForestBuilder, frame: &FrameId) -> Option<usize> {
    let parent = page.inner.lock_state().frames.get(frame)?.parent.clone()?;
    builder.positions.get(&parent).copied()
}

fn stamp_unchanged(page: &Page, frame: &FrameId, before: &FrameStamp) -> bool {
    frame_of(page, frame.clone()).stamp().ok().as_ref() == Some(before)
}

fn loader_changed(method: &str) -> BrowserError {
    BrowserError::LoaderChanged {
        method: method.to_owned(),
    }
}

pub(crate) fn page_session_gone(page: &Page) -> bool {
    page.is_closed()
        || page
            .inner
            .connection
            .session_info(&page.inner.session)
            .is_none_or(|info| info.detached)
}

fn page_unreachable(page: &Page, session: &SessionId, error: &BrowserError) -> bool {
    match error {
        BrowserError::Disconnected { .. } => true,
        BrowserError::TargetCrashed { target_id } => target_id == page.target_id().as_str(),
        _ => {
            error.class() == ErrorClass::SessionGone
                && (*session == page.inner.session || page_session_gone(page))
        }
    }
}

fn restarts_snapshot(page: &Page, session: &SessionId, error: &BrowserError) -> bool {
    page_unreachable(page, session, error)
        || matches!(
            error.class(),
            ErrorClass::AbortedByNavigation | ErrorClass::NoSuchExecutionContext
        )
}

/// Creates no remote objects (every read returns backend node ids), so there is no object group to release.
async fn capture_forest(page: &Page, options: SnapshotOptions) -> crate::Result<AxForest> {
    let (main, children, omitted) = forest_frames(page, options.max_child_frames);
    let mut builder = ForestBuilder::new(page.target_id().clone());
    let stamp = frame_of(page, main.clone()).stamp()?;
    let read = read_frame(page, &mut builder, &main, &stamp).await?;
    builder.push(&main, stamp, read, None);
    capture_children(page, &mut builder, children).await?;
    if omitted > 0 {
        builder.forest.warnings.push(format!(
            "The snapshot stops after {} child frames; {omitted} more were left out",
            options.max_child_frames
        ));
    }
    if options.capture_dom_snapshots {
        capture_dom_snapshots(page, &mut builder).await?;
    }
    Ok(builder.forest)
}

async fn capture_children(
    page: &Page,
    builder: &mut ForestBuilder,
    children: Vec<FrameId>,
) -> crate::Result<()> {
    for child in children {
        capture_child(page, builder, child).await?;
        builder.main_unchanged(page, "Accessibility.getFullAXTree")?;
    }
    Ok(())
}

async fn capture_child(
    page: &Page,
    builder: &mut ForestBuilder,
    frame: FrameId,
) -> crate::Result<()> {
    let stamp = match frame_of(page, frame.clone()).stamp() {
        Ok(stamp) => stamp,
        Err(error) => {
            builder.skip(&frame, error);
            return Ok(());
        }
    };
    let Some(parent) = parent_position(page, builder, &frame) else {
        builder.skip(&frame, "its parent frame is not part of the snapshot");
        return Ok(());
    };
    let read = match read_frame(page, builder, &frame, &stamp).await {
        Ok(read) => read,
        Err(error) if restarts_snapshot(page, &stamp.session, &error) => return Err(error),
        Err(error) => {
            builder.skip(&frame, error);
            return Ok(());
        }
    };
    let parent_frame = builder.forest.frames[parent].frame_id.clone();
    let parent_stamp = builder.stamps[parent].clone();
    match frame_owner(page, &parent_frame, &parent_stamp, &frame).await {
        Ok(backend_node_id) => {
            let owner = FrameOwner {
                parent,
                backend_node_id,
            };
            builder.push(&frame, stamp, read, Some(owner));
            Ok(())
        }
        Err(error) if page_unreachable(page, &parent_stamp.session, &error) => Err(error),
        Err(error) => {
            builder.skip(&frame, error);
            Ok(())
        }
    }
}

async fn read_frame(
    page: &Page,
    builder: &mut ForestBuilder,
    frame: &FrameId,
    before: &FrameStamp,
) -> crate::Result<FrameRead> {
    let session = page.inner.connection.session(Some(before.session.clone()));
    if builder.accessibility_enabled.insert(before.session.clone())
        && let Err(error) = session.send("Accessibility.enable", json!({})).await
    {
        tracing::debug!(target: LOG_TARGET, session = %before.session, %error, "Accessibility.enable failed");
    }
    let mut tree = session
        .send("Accessibility.getFullAXTree", json!({"frameId": frame}))
        .await?;
    let (url, url_fragment) = document_url(page, frame);
    if !stamp_unchanged(page, frame, before) {
        return Err(loader_changed("Accessibility.getFullAXTree"));
    }
    let nodes = match tree.get_mut("nodes").map(Value::take) {
        Some(nodes @ Value::Array(_)) => nodes,
        _ => Value::Array(Vec::new()),
    };
    Ok(FrameRead {
        nodes,
        url,
        url_fragment,
    })
}

async fn frame_owner(
    page: &Page,
    parent: &FrameId,
    parent_stamp: &FrameStamp,
    frame: &FrameId,
) -> crate::Result<i64> {
    let session = page
        .inner
        .connection
        .session(Some(parent_stamp.session.clone()));
    let owner = session
        .send("DOM.getFrameOwner", json!({"frameId": frame}))
        .await?;
    if !stamp_unchanged(page, parent, parent_stamp) {
        return Err(loader_changed("DOM.getFrameOwner"));
    }
    owner["backendNodeId"]
        .as_i64()
        .ok_or_else(|| BrowserError::NotFound {
            message: format!("DOM.getFrameOwner named no owner element for frame {frame}"),
        })
}

async fn capture_dom_snapshots(page: &Page, builder: &mut ForestBuilder) -> crate::Result<()> {
    for (root_frame, local_root, stamp) in builder.local_roots() {
        let session = page.inner.connection.session(Some(stamp.session.clone()));
        if let Err(error) = session.send("DOMSnapshot.enable", json!({})).await {
            tracing::debug!(target: LOG_TARGET, session = %stamp.session, %error, "DOMSnapshot.enable failed");
        }
        let captured = session
            .send(
                "DOMSnapshot.captureSnapshot",
                json!({"computedStyles": ["cursor"]}),
            )
            .await
            .and_then(|snapshot| {
                if stamp_unchanged(page, &root_frame, &stamp) {
                    Ok(snapshot)
                } else {
                    Err(loader_changed("DOMSnapshot.captureSnapshot"))
                }
            });
        match captured {
            Ok(snapshot) => {
                builder.forest.dom_snapshots.insert(local_root, snapshot);
            }
            Err(error) if page_unreachable(page, &stamp.session, &error) => return Err(error),
            Err(error) => builder.forest.warnings.push(format!(
                "Browser snapshot skipped clickable-element detection in frame {root_frame}: {error}"
            )),
        }
    }
    builder.main_unchanged(page, "DOMSnapshot.captureSnapshot")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{PageHarness, default_auto_reply};
    use crate::transport::memory::SentCommand;
    use std::time::Duration;
    use tokio::task::JoinHandle;

    const WAIT: Duration = Duration::from_secs(5);

    fn replying_except(
        withheld: fn(&SentCommand) -> bool,
    ) -> impl Fn(&SentCommand) -> Option<Value> + Send + Sync + use<> {
        move |command| {
            if withheld(command) {
                return None;
            }
            match command.method.as_str() {
                "Accessibility.getFullAXTree" => Some(json!({"nodes": []})),
                "DOM.getFrameOwner" => Some(json!({"backendNodeId": 7, "nodeId": 0})),
                "DOMSnapshot.captureSnapshot" => Some(json!({"documents": [], "strings": []})),
                _ => default_auto_reply(command),
            }
        }
    }

    fn tree_read_of(frame: &str, command: &SentCommand) -> bool {
        command.method == "Accessibility.getFullAXTree" && command.params["frameId"] == frame
    }

    fn navigated(frame: &str, parent: Option<&str>, loader: &str) -> Value {
        let url = format!("http://127.0.0.1/{loader}");
        json!({"frame": {"id": frame, "parentId": parent, "loaderId": loader, "url": url}, "type": "Navigation"})
    }

    /// T1 (S1) with the same-process child F1 and the cross-process child F2 (S2).
    async fn framed_harness() -> PageHarness {
        let harness = PageHarness::new().await;
        harness.emit(
            "Page.frameAttached",
            json!({"frameId": "F1", "parentFrameId": "T1"}),
        );
        harness.emit("Page.frameNavigated", navigated("F1", Some("T1"), "LF1"));
        let child = FrameId::from("F1");
        let deadline = tokio::time::Instant::now() + WAIT;
        let committed = harness
            .page
            .wait_for_state(deadline, |state| state.frames.committed_loader(&child))
            .await;
        assert_eq!(committed, Some(LoaderId::from("LF1")));
        harness.attach_child("S2", "F2", "T1", "https://cross.test/", "LF2");
        harness
    }

    fn spawn_capture(harness: &PageHarness) -> JoinHandle<crate::Result<AxForest>> {
        let page = harness.page.clone();
        tokio::spawn(async move { capture_forest(&page, SnapshotOptions::default()).await })
    }

    async fn outcome(task: JoinHandle<crate::Result<AxForest>>) -> crate::Result<AxForest> {
        tokio::time::timeout(WAIT, task)
            .await
            .expect("the capture never finished")
            .unwrap()
    }

    fn frame_ids(forest: &AxForest) -> Vec<&str> {
        forest
            .frames
            .iter()
            .map(|frame| frame.frame_id.as_str())
            .collect()
    }

    fn assert_loader_changed(outcome: &crate::Result<AxForest>, expected: &str) {
        assert!(
            matches!(outcome, Err(BrowserError::LoaderChanged { method }) if method == expected),
            "{:?}",
            outcome.as_ref().map(frame_ids)
        );
    }

    #[tokio::test]
    async fn a_document_change_during_a_tree_read_is_loader_changed() {
        let mut harness = PageHarness::new().await;
        harness
            .control
            .set_auto_reply(replying_except(|command| tree_read_of("T1", command)));
        let capture = spawn_capture(&harness);
        let read = harness
            .control
            .wait_for("Accessibility.getFullAXTree", Some("S1"))
            .await;
        harness.emit("Page.frameNavigated", navigated("T1", None, "L2"));
        harness.control.reply(&read, json!({"nodes": []}));
        assert_loader_changed(&outcome(capture).await, "Accessibility.getFullAXTree");
    }

    #[tokio::test]
    async fn a_main_document_change_during_a_dom_snapshot_restarts_the_snapshot() {
        let mut harness = PageHarness::new().await;
        harness.control.set_auto_reply(replying_except(|command| {
            command.method == "DOMSnapshot.captureSnapshot"
        }));
        let capture = spawn_capture(&harness);
        let snapshot = harness
            .control
            .wait_for("DOMSnapshot.captureSnapshot", Some("S1"))
            .await;
        harness.emit("Page.frameNavigated", navigated("T1", None, "L2"));
        harness
            .control
            .reply(&snapshot, json!({"documents": [], "strings": []}));
        assert_loader_changed(&outcome(capture).await, "DOMSnapshot.captureSnapshot");
    }

    #[tokio::test]
    async fn a_main_document_change_during_an_owner_lookup_restarts_the_snapshot() {
        let mut harness = framed_harness().await;
        harness.control.set_auto_reply(replying_except(|command| {
            command.method == "DOM.getFrameOwner" && command.params["frameId"] == "F1"
        }));
        let capture = spawn_capture(&harness);
        let lookup = harness
            .control
            .wait_for("DOM.getFrameOwner", Some("S1"))
            .await;
        harness.emit("Page.frameNavigated", navigated("T1", None, "L2"));
        harness
            .control
            .reply(&lookup, json!({"backendNodeId": 7, "nodeId": 0}));
        assert_loader_changed(&outcome(capture).await, "Accessibility.getFullAXTree");
    }

    #[tokio::test]
    async fn a_child_document_change_during_its_tree_read_restarts_the_snapshot() {
        let mut harness = framed_harness().await;
        harness
            .control
            .set_auto_reply(replying_except(|command| tree_read_of("F1", command)));
        let capture = spawn_capture(&harness);
        let read = harness
            .control
            .wait_for("Accessibility.getFullAXTree", Some("S1"))
            .await;
        assert_eq!(read.params["frameId"], "F1");
        harness.emit("Page.frameNavigated", navigated("F1", Some("T1"), "LF1b"));
        harness.control.reply(&read, json!({"nodes": []}));
        assert_loader_changed(&outcome(capture).await, "Accessibility.getFullAXTree");
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum PageLoss {
        Crash,
        Detach,
        Disconnect,
    }

    fn lose_page(harness: &PageHarness, loss: PageLoss) {
        match loss {
            PageLoss::Crash => harness.emit("Inspector.targetCrashed", json!({})),
            PageLoss::Detach => {
                let params = json!({"sessionId": "S1", "targetId": "T1"});
                harness
                    .control
                    .emit("Target.detachedFromTarget", None, params);
            }
            PageLoss::Disconnect => harness.control.close("the browser exited"),
        }
    }

    fn loss_reported_by(outcome: &crate::Result<AxForest>) -> Option<PageLoss> {
        match outcome {
            Err(BrowserError::TargetCrashed { target_id }) if target_id == "T1" => {
                Some(PageLoss::Crash)
            }
            Err(BrowserError::TargetClosed { .. }) => Some(PageLoss::Detach),
            Err(BrowserError::Disconnected { .. }) => Some(PageLoss::Disconnect),
            _ => None,
        }
    }

    #[tokio::test]
    async fn losing_the_page_during_a_child_read_fails_the_snapshot() {
        for loss in [PageLoss::Crash, PageLoss::Detach, PageLoss::Disconnect] {
            let mut harness = framed_harness().await;
            harness
                .control
                .set_auto_reply(replying_except(|command| tree_read_of("F2", command)));
            let capture = spawn_capture(&harness);
            harness
                .control
                .wait_for("Accessibility.getFullAXTree", Some("S2"))
                .await;
            lose_page(&harness, loss);
            let outcome = outcome(capture).await;
            assert_eq!(
                loss_reported_by(&outcome),
                Some(loss),
                "{:?}",
                outcome.as_ref().map(frame_ids)
            );
        }
    }

    #[tokio::test]
    async fn a_crashed_cross_process_frame_is_left_out() {
        let mut harness = framed_harness().await;
        harness
            .control
            .set_auto_reply(replying_except(|command| tree_read_of("F2", command)));
        let capture = spawn_capture(&harness);
        harness
            .control
            .wait_for("Accessibility.getFullAXTree", Some("S2"))
            .await;
        harness.emit_on("S2", "Inspector.targetCrashed", json!({}));
        let forest = outcome(capture).await.unwrap();
        assert_eq!(frame_ids(&forest), ["T1", "F1"]);
        assert_eq!(
            forest.warnings,
            [
                "Frame F2 was left out of the snapshot: The page crashed (target F2); reload it or open a new page"
            ]
        );
    }

    fn reads_value(command: &SentCommand) -> bool {
        command.method == "Runtime.callFunctionOn"
            && command.params["arguments"][0]["value"] == "value"
    }

    fn withholding_the_value_read(command: &SentCommand) -> Option<Value> {
        match command.method.as_str() {
            _ if reads_value(command) => None,
            "Runtime.callFunctionOn" => Some(json!({"result": {"type": "boolean", "value": true}})),
            "DOM.resolveNode" => Some(json!({"object": {"type": "object", "objectId": "node-1"}})),
            _ => replying_except(|_| false)(command),
        }
    }

    async fn released_groups(harness: &PageHarness) -> Vec<Value> {
        let is_release = |command: &SentCommand| command.method == "Runtime.releaseObjectGroup";
        harness.sent("the op release", is_release).await;
        harness
            .control
            .commands_seen()
            .into_iter()
            .filter(is_release)
            .map(|command| command.params["objectGroup"].clone())
            .collect()
    }

    #[tokio::test]
    async fn a_snapshot_during_an_element_op_leaves_the_op_objects_alive() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(withholding_the_value_read);
        let element = crate::testing::element(&harness.page, "T1", 42).unwrap();
        let read = tokio::spawn(async move { element.property("value").await });
        let call = harness.sent("the element read", reads_value).await;
        assert_eq!(call.params["objectId"], "node-1");
        let forest = harness
            .page
            .accessibility_forest(SnapshotOptions::default());
        tokio::time::timeout(WAIT, forest)
            .await
            .expect("the snapshot never finished")
            .unwrap();
        harness.control.reply(
            &call,
            json!({"result": {"type": "string", "value": "typed"}}),
        );
        let value = tokio::time::timeout(WAIT, read)
            .await
            .expect("the element read never finished")
            .unwrap();
        assert_eq!(value.unwrap(), json!("typed"));
        let resolved = harness
            .sent("the node resolution", |command| {
                command.method == "DOM.resolveNode"
            })
            .await;
        assert_eq!(
            released_groups(&harness).await,
            [resolved.params["objectGroup"].clone()],
            "the snapshot releases nothing and the op releases only its own group"
        );
    }

    #[test]
    fn default_options_cover_twenty_child_frames_with_dom_snapshots() {
        let options = SnapshotOptions::default();
        assert_eq!(options.max_child_frames, 20);
        assert!(options.capture_dom_snapshots);
    }
}
