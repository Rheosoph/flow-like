// Derived from agent-browser cli/src/native/interaction.rs @d01253d, Copyright 2025 Vercel Inc., Apache-2.0, and Chromium chrome/test/chromedriver element_commands.cc, element_util.cc, window_commands.cc @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE.
use serde::Deserialize;
use serde_json::{Value, json};

use crate::connection::PendingReply;
use crate::dialogs::Dialog;
use crate::element::{Atom, Element, ResolvedNode};
use crate::error::{BrowserError, ErrorClass};
use crate::input::{
    ActionOutcome, ClickOptions, InputEvent, InputGuard, MouseButton, MouseEvent, dialog_outcome,
    record_sent, unexpected_reply,
};
use crate::page::{Frame, FrameStamp, Page, PageEffect};
use crate::script::World;
use crate::session::Session;
use crate::settle::{OpAttempt, OpSpec};
use crate::types::{FrameId, SessionId};

const MIN_QUAD_AREA: f64 = 0.99;
const CLICK_TARGET_PROBE: &str = "function(){var t=this.tagName.toLowerCase();return {tag:t,type:t==='input'?String(this.type).toLowerCase():''}}";
const HIT_CHECK: &str = "function(h){for(var n=h;n;n=n.assignedSlot||n.parentNode||(n.nodeType===11?n.host:null)){if(n===this)return true}return false}";
const NO_LAYOUT: &str = "element has no size or is not displayed";
const OUTSIDE_VIEWPORT: &str = "element is outside the viewport";
const DRAG_APART: &str = "the drag source and target do not fit in the viewport together";
const QUADS: &str = "DOM.getContentQuads";
const HIT_TEST: &str = "DOM.getNodeForLocation";
const PRESENTED_FRAME: &str = "new Promise(function(done){requestAnimationFrame(function(){requestAnimationFrame(function(){done(true)})})})";
const PRESENTED_FRAME_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

type FrameChain = Vec<(FrameId, FrameStamp)>;

static OPTION_TOGGLEABLE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "function(){{{}\nreturn isOptionElementToggleable(this);}}",
        include_str!("../js/is_option_element_toggleable.js")
    )
});

pub(crate) struct Boundary {
    pub session: crate::session::Session,
    pub owner_backend_node_id: i64,
    pub point: (f64, f64),
}

pub(crate) struct ClickPoint {
    pub session: crate::session::Session,
    pub local: (f64, f64),
    pub top: (f64, f64),
    pub boundaries: Vec<Boundary>,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingRelease {
    pub session: SessionId,
    pub x: f64,
    pub y: f64,
    pub button: MouseButton,
    pub click_count: u8,
}

impl PendingRelease {
    pub(crate) fn is_held(&self, session: &SessionId, button: MouseButton) -> bool {
        self.session == *session && self.button == button
    }

    pub(crate) fn effect(&self) -> PageEffect {
        let released = MouseEvent::released((self.x, self.y), self.button, self.click_count, 0);
        PageEffect::SendNowait {
            session: self.session.clone(),
            method: "Input.dispatchMouseEvent",
            params: released.params(),
        }
    }
}

pub(crate) enum Dispatch {
    Acknowledged,
    Dialog(Dialog),
    TargetGone,
}

impl Dispatch {
    fn finished(self) -> Option<ActionOutcome> {
        match self {
            Dispatch::Acknowledged => None,
            Dispatch::Dialog(dialog) => Some(dialog_outcome(dialog)),
            Dispatch::TargetGone => Some(ActionOutcome::Completed),
        }
    }
}

struct Hit {
    node: i64,
    frame: FrameId,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ClickTarget {
    tag: String,
    #[serde(rename = "type")]
    type_: String,
}

impl ClickTarget {
    fn is_file_input(&self) -> bool {
        self.tag == "input" && self.type_ == "file"
    }
}

#[derive(Clone, Copy, Debug)]
struct Viewport {
    width: f64,
    height: f64,
    scroll: (f64, f64),
}

impl Viewport {
    fn contains(self, (x, y): (f64, f64)) -> bool {
        (0.0..=self.width).contains(&x) && (0.0..=self.height).contains(&y)
    }

    fn clip(self, quad: [(f64, f64); 4]) -> [(f64, f64); 4] {
        quad.map(|(x, y)| (x.clamp(0.0, self.width), y.clamp(0.0, self.height)))
    }

    fn document_point(self, (x, y): (f64, f64)) -> (f64, f64) {
        (x + self.scroll.0, y + self.scroll.1)
    }
}

pub(crate) fn element_stamp(element: &Element) -> crate::Result<FrameStamp> {
    let stamp = element.frame.stamp()?;
    if stamp.loader != element.loader_id || stamp.local_root != element.local_root {
        return Err(BrowserError::StaleElement);
    }
    Ok(stamp)
}

pub(crate) fn committed_error(attempt: &OpAttempt, error: BrowserError) -> crate::Result<Dispatch> {
    if attempt.is_committed() && error.class() == ErrorClass::SessionGone {
        Ok(Dispatch::TargetGone)
    } else {
        Err(error)
    }
}

pub(crate) async fn dispatch_or_dialog(
    page: &Page,
    attempt: &OpAttempt,
    session: &Session,
    event: &InputEvent,
) -> crate::Result<Dispatch> {
    let dialogs = page.dialog_count();
    if let Some(dialog) = page.pending_dialog() {
        return Ok(Dispatch::Dialog(dialog));
    }
    let pending = match session.enqueue(event.method(), event.params(), None) {
        Ok(pending) => pending,
        Err(error) => return committed_error(attempt, error),
    };
    attempt.commit();
    if let Some(id) = session.id() {
        record_sent(page, id, event, pending.ticket(), attempt.deadline);
    }
    await_or_dialog(page, attempt, dialogs, pending).await
}

pub(crate) async fn await_or_dialog(
    page: &Page,
    attempt: &OpAttempt,
    dialogs: u64,
    mut pending: PendingReply,
) -> crate::Result<Dispatch> {
    tokio::select! {
        biased;
        reply = &mut pending => match reply {
            Ok(_) => Ok(Dispatch::Acknowledged),
            Err(error) => committed_error(attempt, error),
        },
        dialog = page.dialog_opened_after(dialogs) => {
            tokio::spawn(async move {
                if let Err(error) = pending.await {
                    tracing::debug!(%error, "input event reply after a dialog opened");
                }
            });
            Ok(Dispatch::Dialog(dialog))
        }
    }
}

async fn dispatch_all(
    page: &Page,
    attempt: &OpAttempt,
    session: &Session,
    events: &[InputEvent],
) -> crate::Result<ActionOutcome> {
    for event in events {
        let sent = dispatch_or_dialog(page, attempt, session, event).await?;
        if let Some(outcome) = sent.finished() {
            return Ok(outcome);
        }
    }
    Ok(ActionOutcome::Completed)
}

pub(crate) async fn w3c_click(
    page: &Page,
    attempt: &OpAttempt,
    point: (f64, f64),
    options: ClickOptions,
) -> crate::Result<ActionOutcome> {
    let downs = crate::input::keys::modifier_events(options.modifiers, true);
    let ups = crate::input::keys::modifier_events(options.modifiers, false);
    let modifiers = options.modifiers.bits();
    let mut events: Vec<InputEvent> = downs.into_iter().map(InputEvent::Key).collect();
    events.push(InputEvent::Mouse(MouseEvent::moved(point, None, modifiers)));
    for count in 1..=options.click_count {
        let pressed = MouseEvent::pressed(point, options.button, count, modifiers);
        let released = MouseEvent::released(point, options.button, count, modifiers);
        events.extend([InputEvent::Mouse(pressed), InputEvent::Mouse(released)]);
    }
    events.extend(ups.into_iter().map(InputEvent::Key));
    dispatch_all(page, attempt, &page.session(), &events).await
}

fn quads_of(reply: &Value) -> Vec<[(f64, f64); 4]> {
    let Some(quads) = reply["quads"].as_array() else {
        return Vec::new();
    };
    quads
        .iter()
        .filter_map(|quad| {
            let numbers: Vec<f64> = quad.as_array()?.iter().filter_map(Value::as_f64).collect();
            let points: [f64; 8] = numbers.try_into().ok()?;
            Some([
                (points[0], points[1]),
                (points[2], points[3]),
                (points[4], points[5]),
                (points[6], points[7]),
            ])
        })
        .collect()
}

fn area(quad: &[(f64, f64); 4]) -> f64 {
    let twice: f64 = (0..4)
        .map(|index| {
            let (x1, y1) = quad[index];
            let (x2, y2) = quad[(index + 1) % 4];
            x1 * y2 - x2 * y1
        })
        .sum();
    (twice / 2.0).abs()
}

fn quad_centre(reply: &Value, viewport: Viewport) -> crate::Result<(f64, f64)> {
    let sized: Vec<_> = quads_of(reply)
        .into_iter()
        .filter(|quad| area(quad) > MIN_QUAD_AREA)
        .collect();
    if sized.is_empty() {
        return Err(not_interactable(NO_LAYOUT));
    }
    let visible = sized
        .into_iter()
        .map(|quad| viewport.clip(quad))
        .find(|quad| area(quad) > MIN_QUAD_AREA)
        .ok_or_else(|| not_interactable(OUTSIDE_VIEWPORT))?;
    let (x, y) = visible.iter().fold((0.0, 0.0), |(x, y), point| {
        (x + point.0 / 4.0, y + point.1 / 4.0)
    });
    Ok((x, y))
}

fn not_interactable(message: &str) -> BrowserError {
    BrowserError::NotInteractable {
        message: message.to_owned(),
    }
}

fn drag_apart(error: BrowserError) -> BrowserError {
    match &error {
        BrowserError::NotInteractable { message } if message == OUTSIDE_VIEWPORT => {
            not_interactable(DRAG_APART)
        }
        _ => error,
    }
}

fn node_error(error: BrowserError) -> BrowserError {
    match &error {
        BrowserError::Protocol { message, .. }
            if message.contains("layout object") || message.contains("compute box model") =>
        {
            not_interactable(NO_LAYOUT)
        }
        _ if error.class() == ErrorClass::NodeGone => BrowserError::StaleElement,
        _ => error,
    }
}

async fn layout_viewport(session: &Session) -> crate::Result<Viewport> {
    let method = "Page.getLayoutMetrics";
    let metrics = session.send(method, json!({})).await?;
    let viewport = &metrics["cssLayoutViewport"];
    let field = |name: &str| viewport[name].as_f64();
    match (
        field("clientWidth"),
        field("clientHeight"),
        field("pageX"),
        field("pageY"),
    ) {
        (Some(width), Some(height), Some(left), Some(top)) => Ok(Viewport {
            width,
            height,
            scroll: (left, top),
        }),
        _ => Err(unexpected_reply(method, "cssLayoutViewport", &metrics)),
    }
}

/// `point` is in the viewport of the local root of `session`; Blink hit-tests document
/// coordinates, so the current scroll offset of that local root is added right before the hit test.
async fn node_at(session: &Session, point: (f64, f64)) -> crate::Result<Hit> {
    let (x, y) = layout_viewport(session).await?.document_point(point);
    let params =
        json!({"x": x.round() as i64, "y": y.round() as i64, "includeUserAgentShadowDOM": true});
    let hit = session.send(HIT_TEST, params).await?;
    match (hit["backendNodeId"].as_i64(), hit["frameId"].as_str()) {
        (Some(node), Some(frame)) => Ok(Hit {
            node,
            frame: FrameId::new(frame),
        }),
        _ => Err(unexpected_reply(HIT_TEST, "backendNodeId or frameId", &hit)),
    }
}

async fn describe_node(session: &Session, backend_node_id: i64) -> crate::Result<String> {
    let described = session
        .send(
            "DOM.describeNode",
            json!({"backendNodeId": backend_node_id}),
        )
        .await?;
    let node = &described["node"];
    let name = node["localName"]
        .as_str()
        .filter(|name| !name.is_empty())
        .or_else(|| node["nodeName"].as_str())
        .unwrap_or("node")
        .to_ascii_lowercase();
    let attributes: Vec<&str> = node["attributes"]
        .as_array()
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let attribute = |wanted: &str| {
        attributes
            .as_chunks::<2>()
            .0
            .iter()
            .find(|[name, _]| *name == wanted)
            .map(|[_, value]| *value)
    };
    let mut label = name;
    if let Some(id) = attribute("id").filter(|id| !id.is_empty()) {
        label.push('#');
        label.push_str(id);
    }
    for class in attribute("class").unwrap_or_default().split_whitespace() {
        label.push('.');
        label.push_str(class);
    }
    Ok(label)
}

async fn intercepted(session: &Session, hit: i64, top: (f64, f64), tag: &str) -> BrowserError {
    let other = describe_node(session, hit)
        .await
        .unwrap_or_else(|_| format!("node {hit}"));
    BrowserError::ClickIntercepted {
        message: format!(
            "Element <{tag}> is not clickable at point ({:.0}, {:.0}). Other element would receive the click: <{other}>",
            top.0, top.1
        ),
    }
}

fn parent_of(page: &Page, frame: &FrameId) -> Option<FrameId> {
    page.inner
        .lock_state()
        .frames
        .get(frame)
        .and_then(|node| node.parent.clone())
}

fn frame_chain(frame: &Frame) -> crate::Result<FrameChain> {
    let mut chain = vec![(frame.id.clone(), frame.stamp()?)];
    while let Some(parent) = chain.last().and_then(|(id, _)| parent_of(&frame.page, id)) {
        let link = Frame {
            page: frame.page.clone(),
            id: parent,
        };
        let stamp = link.stamp()?;
        chain.push((link.id, stamp));
    }
    Ok(chain)
}

fn element_chain(element: &Element) -> crate::Result<FrameChain> {
    let chain = frame_chain(&element.frame)?;
    let own = &chain[0].1;
    if own.loader != element.loader_id || own.local_root != element.local_root {
        return Err(BrowserError::StaleElement);
    }
    Ok(chain)
}

fn unchanged(frame: &Frame, before: &FrameChain, method: &str) -> crate::Result<()> {
    if frame_chain(frame)? != *before {
        return Err(BrowserError::LoaderChanged {
            method: method.to_owned(),
        });
    }
    Ok(())
}

fn parent_link(chain: &FrameChain, root: &FrameId) -> crate::Result<Option<FrameStamp>> {
    let index =
        chain
            .iter()
            .position(|(id, _)| id == root)
            .ok_or_else(|| BrowserError::NoSuchFrame {
                message: format!(
                    "The local root {root} is not an ancestor of the frame {}",
                    chain[0].0
                ),
            })?;
    Ok(chain.get(index + 1).map(|(_, stamp)| stamp.clone()))
}

fn child_toward(page: &Page, from: &FrameId, ancestor: &FrameId) -> Option<FrameId> {
    let mut current = from.clone();
    loop {
        let parent = parent_of(page, &current)?;
        if parent == *ancestor {
            return Some(current);
        }
        current = parent;
    }
}

async fn frame_owner(session: &Session, frame: &FrameId) -> crate::Result<i64> {
    let method = "DOM.getFrameOwner";
    let owner = session.send(method, json!({"frameId": frame})).await?;
    owner["backendNodeId"]
        .as_i64()
        .ok_or_else(|| unexpected_reply(method, "backendNodeId", &owner))
}

async fn owner_origin(parent: &Session, frame: &FrameId) -> crate::Result<(i64, (f64, f64))> {
    let owner = frame_owner(parent, frame).await?;
    let method = "DOM.getBoxModel";
    let model = parent
        .send(method, json!({"backendNodeId": owner}))
        .await
        .map_err(node_error)?;
    let content = &model["model"]["content"];
    match (content[0].as_f64(), content[1].as_f64()) {
        (Some(x), Some(y)) => Ok((owner, (x, y))),
        _ => Err(unexpected_reply(method, "model.content", &model)),
    }
}

async fn walk_to_top(
    page: &Page,
    chain: &FrameChain,
    local: (f64, f64),
) -> crate::Result<((f64, f64), Vec<Boundary>)> {
    let mut root = FrameId::new(chain[0].1.local_root.as_str());
    let mut point = local;
    let mut boundaries = Vec::new();
    while let Some(parent) = parent_link(chain, &root)? {
        let session = page.inner.connection.session(Some(parent.session.clone()));
        let (owner, origin) = owner_origin(&session, &root).await?;
        point = (point.0 + origin.0, point.1 + origin.1);
        boundaries.push(Boundary {
            session,
            owner_backend_node_id: owner,
            point,
        });
        root = FrameId::new(parent.local_root.as_str());
    }
    Ok((point, boundaries))
}

async fn resolve_hit(attempt: &OpAttempt, node: &ResolvedNode, hit: i64) -> crate::Result<String> {
    let method = "DOM.resolveNode";
    let group = attempt.objects.on(&node.session);
    let params =
        json!({"backendNodeId": hit, "executionContextId": node.context.id, "objectGroup": group});
    let resolved = node.session.send(method, params).await?;
    resolved["object"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| unexpected_reply(method, "object.objectId", &resolved))
}

fn script_value(reply: &Value) -> crate::Result<&Value> {
    let Some(details) = reply.get("exceptionDetails") else {
        return Ok(&reply["result"]["value"]);
    };
    let message = details["exception"]["description"]
        .as_str()
        .or_else(|| details["text"].as_str())
        .unwrap_or("the script threw");
    Err(BrowserError::Javascript {
        message: message.to_owned(),
    })
}

async fn scroll_into_view(session: &Session, backend_node_id: i64) -> crate::Result<()> {
    let node = json!({"backendNodeId": backend_node_id});
    session
        .send("DOM.scrollIntoViewIfNeeded", node)
        .await
        .map_err(node_error)?;
    Ok(())
}

/// Page-session input is routed through the browser's hit-test data, which a frame only updates
/// once it presented a compositor frame: right after a process swap a click into an OOPIF can
/// land in the parent, and right after a scroll or layout change a click on a page with OOPIFs
/// can land in an iframe that was at the point before. Bounded, because a hidden page runs no
/// animation frames.
async fn await_presented_frame(node: &ResolvedNode) {
    let params =
        json!({"expression": PRESENTED_FRAME, "awaitPromise": true, "contextId": node.context.id});
    let presented = node
        .session
        .send_with_timeout("Runtime.evaluate", params, PRESENTED_FRAME_WAIT)
        .await;
    if let Err(error) = presented {
        tracing::debug!(%error, "no presented frame before an OOPIF pointer action");
    }
}

async fn local_centre(session: &Session, backend_node_id: i64) -> crate::Result<(f64, f64)> {
    let quads = async {
        let quads = session
            .send(QUADS, json!({"backendNodeId": backend_node_id}))
            .await;
        quads.map_err(node_error)
    };
    let (quads, viewport) = tokio::try_join!(quads, layout_viewport(session))?;
    quad_centre(&quads, viewport)
}

async fn top_point(
    page: &Page,
    chain: &FrameChain,
    local: (f64, f64),
) -> crate::Result<((f64, f64), Vec<Boundary>)> {
    let (top, boundaries) = walk_to_top(page, chain, local).await?;
    if !boundaries.is_empty() && !layout_viewport(&page.session()).await?.contains(top) {
        return Err(not_interactable(OUTSIDE_VIEWPORT));
    }
    Ok((top, boundaries))
}

async fn contains_hit(element: &Element, attempt: &OpAttempt, hit: i64) -> crate::Result<bool> {
    if hit == element.backend_node_id {
        return Ok(true);
    }
    let node = element.resolve_in(attempt, World::Util).await?;
    let hit_object = resolve_hit(attempt, &node, hit).await?;
    let params = json!({
        "objectId": node.object_id,
        "functionDeclaration": HIT_CHECK,
        "arguments": [{"objectId": hit_object}],
        "returnByValue": true,
    });
    let reply = node.session.send("Runtime.callFunctionOn", params).await?;
    Ok(*script_value(&reply)? == Value::Bool(true))
}

async fn hit_in_element_document(
    element: &Element,
    session: &Session,
    hit: &Hit,
) -> crate::Result<Option<i64>> {
    if hit.frame == element.frame.id {
        return Ok(Some(hit.node));
    }
    match child_toward(&element.frame.page, &hit.frame, &element.frame.id) {
        Some(child) => frame_owner(session, &child).await.map(Some),
        None => Ok(None),
    }
}

async fn local_interceptor(
    element: &Element,
    attempt: &OpAttempt,
    point: &ClickPoint,
) -> crate::Result<Option<i64>> {
    let hit = node_at(&point.session, point.local).await?;
    let Some(node) = hit_in_element_document(element, &point.session, &hit).await? else {
        return Ok(Some(hit.node));
    };
    let reaches = contains_hit(element, attempt, node).await?;
    Ok((!reaches).then_some(node))
}

async fn boundary_interceptor(point: &ClickPoint) -> crate::Result<Option<(&Session, i64)>> {
    for boundary in &point.boundaries {
        let hit = node_at(&boundary.session, boundary.point).await?;
        if hit.node != boundary.owner_backend_node_id {
            return Ok(Some((&boundary.session, hit.node)));
        }
    }
    Ok(None)
}

async fn check_hit_chain(
    element: &Element,
    attempt: &OpAttempt,
    point: &ClickPoint,
    tag: &str,
) -> crate::Result<()> {
    if let Some(other) = local_interceptor(element, attempt, point).await? {
        return Err(intercepted(&point.session, other, point.top, tag).await);
    }
    if let Some((session, other)) = boundary_interceptor(point).await? {
        return Err(intercepted(session, other, point.top, tag).await);
    }
    Ok(())
}

impl Element {
    pub async fn click(&self, options: ClickOptions) -> crate::Result<ActionOutcome> {
        let page = &self.frame.page;
        let _guard = InputGuard::new(page);
        let outcome = self
            .run(OpSpec::INPUT, |attempt| async move {
                let point = self.click_point_in(&attempt).await?;
                w3c_click(page, &attempt, point.top, options).await
            })
            .await?;
        Ok(outcome.or_else(dialog_outcome))
    }

    pub async fn element_click(&self) -> crate::Result<ActionOutcome> {
        let _guard = InputGuard::new(&self.frame.page);
        let outcome = self
            .run(OpSpec::INPUT, |attempt| async move {
                self.element_click_in(&attempt).await
            })
            .await?;
        Ok(outcome.or_else(dialog_outcome))
    }

    pub async fn hover(&self) -> crate::Result<ActionOutcome> {
        let page = &self.frame.page;
        let outcome = self
            .run(OpSpec::INPUT, |attempt| async move {
                let point = self.click_point_in(&attempt).await?;
                let moved = InputEvent::Mouse(MouseEvent::moved(point.top, None, 0));
                dispatch_all(page, &attempt, &page.session(), &[moved]).await
            })
            .await?;
        Ok(outcome.or_else(dialog_outcome))
    }

    pub async fn drag_to(&self, target: &Element) -> crate::Result<ActionOutcome> {
        let page = &self.frame.page;
        if target.frame.page.target_id() != page.target_id() {
            return Err(BrowserError::InvalidArgument {
                message: "the drag target is on another page than the dragged element".to_owned(),
            });
        }
        let _guard = InputGuard::new(page);
        let spec = OpSpec {
            frame_scoped: false,
            ..OpSpec::INPUT
        };
        let outcome = page
            .run_op(&self.frame.id, spec, |attempt| async move {
                self.drag_to_in(&attempt, target).await
            })
            .await?;
        Ok(outcome.or_else(dialog_outcome))
    }

    pub(crate) async fn click_point_in(&self, attempt: &OpAttempt) -> crate::Result<ClickPoint> {
        Ok(self.scrolled_point_in(attempt).await?.0)
    }

    async fn scrolled_point_in(
        &self,
        attempt: &OpAttempt,
    ) -> crate::Result<(ClickPoint, FrameChain)> {
        let node = self.resolve_in(attempt, World::Util).await?;
        let chain = element_chain(self)?;
        scroll_into_view(&node.session, self.backend_node_id).await?;
        if self.local_root != *self.frame.page.target_id() || self.frame.page.has_oopifs() {
            await_presented_frame(&node).await;
        }
        let point = self.point_on(node.session, &chain).await?;
        Ok((point, chain))
    }

    async fn point_on(&self, session: Session, chain: &FrameChain) -> crate::Result<ClickPoint> {
        let local = local_centre(&session, self.backend_node_id).await?;
        let (top, boundaries) = top_point(&self.frame.page, chain, local).await?;
        unchanged(&self.frame, chain, QUADS)?;
        Ok(ClickPoint {
            session,
            local,
            top,
            boundaries,
        })
    }

    async fn element_click_in(&self, attempt: &OpAttempt) -> crate::Result<ActionOutcome> {
        let probed = self
            .call_function_in(attempt, World::Util, CLICK_TARGET_PROBE, Vec::new(), true)
            .await?;
        let target = ClickTarget::deserialize(&probed).unwrap_or_default();
        if target.tag == "option" {
            return self.element_click_option_in(attempt).await;
        }
        if target.is_file_input() {
            return Err(BrowserError::InvalidArgument {
                message: "element click on a file input is not allowed; use Upload File".to_owned(),
            });
        }
        let point = self.hit_checked_point_in(attempt, &target.tag).await?;
        let events = [
            InputEvent::Mouse(MouseEvent::moved(point.local, None, 0)),
            InputEvent::Mouse(MouseEvent::pressed(point.local, MouseButton::Left, 1, 0)),
            InputEvent::Mouse(MouseEvent::released(point.local, MouseButton::Left, 1, 0)),
        ];
        dispatch_all(&self.frame.page, attempt, &point.session, &events).await
    }

    async fn hit_checked_point_in(
        &self,
        attempt: &OpAttempt,
        tag: &str,
    ) -> crate::Result<ClickPoint> {
        let chain = element_chain(self)?;
        let point = self.click_point_in(attempt).await?;
        let checked = check_hit_chain(self, attempt, &point, tag).await;
        unchanged(&self.frame, &chain, HIT_TEST)?;
        checked.map(|()| point)
    }

    async fn element_click_option_in(&self, attempt: &OpAttempt) -> crate::Result<ActionOutcome> {
        let toggleable = self
            .call_function_in(attempt, World::Util, &OPTION_TOGGLEABLE, Vec::new(), true)
            .await?
            == Value::Bool(true);
        let selected = self
            .call_atom_in(attempt, Atom::IsSelected, Vec::new())
            .await?
            == Value::Bool(true);
        let select = !(toggleable && selected);
        let page = &self.frame.page;
        let dialogs = page.dialog_count();
        if let Some(dialog) = page.pending_dialog() {
            return Ok(dialog_outcome(dialog));
        }
        attempt.commit();
        tokio::select! {
            biased;
            clicked = self.call_atom_in(attempt, Atom::Click, vec![json!(select)]) => match clicked {
                Ok(_) => Ok(ActionOutcome::Completed),
                Err(error) => committed_error(attempt, error).map(|_| ActionOutcome::Completed),
            },
            dialog = page.dialog_opened_after(dialogs) => Ok(dialog_outcome(dialog)),
        }
    }

    async fn drag_to_in(
        &self,
        attempt: &OpAttempt,
        target: &Element,
    ) -> crate::Result<ActionOutcome> {
        let page = &self.frame.page;
        let (first, target_chain) = target
            .scrolled_point_in(attempt)
            .await
            .map_err(|error| target.map_stale(error))?;
        let (source, source_chain) = self
            .scrolled_point_in(attempt)
            .await
            .map_err(|error| self.map_stale(error))?;
        // Scrolling the source into view can scroll the target (and its ancestor frames) away
        // from the point measured first, so the target is measured again without scrolling.
        let to = target
            .point_on(first.session, &target_chain)
            .await
            .map_err(|error| drag_apart(target.map_stale(error)))?
            .top;
        unchanged(&self.frame, &source_chain, QUADS)?;
        let from = source.top;
        let events = [
            InputEvent::Mouse(MouseEvent::moved(from, None, 0)),
            InputEvent::Mouse(MouseEvent::pressed(from, MouseButton::Left, 1, 0)),
            InputEvent::Mouse(MouseEvent::moved(to, Some(MouseButton::Left), 0)),
            InputEvent::Mouse(MouseEvent::released(to, MouseButton::Left, 1, 0)),
        ];
        dispatch_all(page, attempt, &page.session(), &events).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::keys::Modifiers;
    use crate::input::scene::{
        self, NESTED_OWNER, OOPIF_OWNER, TARGET, calls_atom, declaration, element, input_events,
        mouse, value, wait_seen,
    };
    use crate::testing::PageHarness;
    use crate::transport::memory::SentCommand;
    use crate::types::LoaderId;

    fn mouse_events(harness: &PageHarness) -> Vec<(String, String, f64, f64)> {
        input_events(&harness.control)
            .iter()
            .filter(|command| command.method == "Input.dispatchMouseEvent")
            .map(|command| {
                let (kind, x, y) = mouse(command);
                let kind = match kind {
                    "mouseMoved" => "moved",
                    "mousePressed" => "pressed",
                    "mouseReleased" => "released",
                    _ => "other",
                };
                (
                    command.session.clone().unwrap_or_default(),
                    kind.to_owned(),
                    x,
                    y,
                )
            })
            .collect()
    }

    fn at(session: &str, kind: &str, (x, y): (f64, f64)) -> (String, String, f64, f64) {
        (session.to_owned(), kind.to_owned(), x, y)
    }

    fn sent_on(harness: &PageHarness, method: &str) -> Vec<SentCommand> {
        harness
            .control
            .commands_seen()
            .into_iter()
            .filter(|command| command.method == method)
            .collect()
    }

    #[tokio::test]
    async fn w3c_click_on_an_oopif_element_uses_the_page_session_at_the_summed_point() {
        let harness = scene::page_with_oopif().await;
        let button = element(&harness, "F2", TARGET);
        let outcome = button.click(ClickOptions::default()).await.unwrap();
        assert_eq!(outcome, ActionOutcome::Completed);
        let top = (64.25 + 51.0, 60.5 + 297.5);
        assert_eq!(
            mouse_events(&harness),
            vec![
                at("S1", "moved", top),
                at("S1", "pressed", top),
                at("S1", "released", top)
            ]
        );
        let owners = sent_on(&harness, "DOM.getFrameOwner");
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].session.as_deref(), Some("S1"));
        assert_eq!(owners[0].params["frameId"], "F2");
        let quads = sent_on(&harness, "DOM.getContentQuads");
        assert_eq!(quads[0].session.as_deref(), Some("S2"));
        let scroll = sent_on(&harness, "DOM.scrollIntoViewIfNeeded");
        assert_eq!(scroll[0].params["backendNodeId"], TARGET);
    }

    fn presented_frame_waits(harness: &PageHarness) -> Vec<Option<String>> {
        sent_on(harness, "Runtime.evaluate")
            .into_iter()
            .filter(|command| command.params["expression"] == PRESENTED_FRAME)
            .map(|command| command.session)
            .collect()
    }

    #[tokio::test]
    async fn oopif_pointer_input_waits_for_a_presented_frame_of_the_local_root_first() {
        let harness = scene::page_with_oopif().await;
        let button = element(&harness, "F2", TARGET);
        button.click(ClickOptions::default()).await.unwrap();
        assert_eq!(presented_frame_waits(&harness), [Some("S2".to_owned())]);
        let seen = harness.control.commands_seen();
        let waited = seen
            .iter()
            .position(|command| command.params["expression"] == PRESENTED_FRAME)
            .unwrap();
        let pressed = seen
            .iter()
            .position(|command| command.method == "Input.dispatchMouseEvent")
            .unwrap();
        assert!(waited < pressed);
        assert_eq!(seen[waited].params["awaitPromise"], true);
        assert!(seen[waited].params["contextId"].is_i64());
    }

    #[tokio::test]
    async fn main_frame_pointer_input_on_a_page_with_oopifs_waits_for_a_presented_frame() {
        let harness = scene::page_with_oopif().await;
        let button = element(&harness, "T1", TARGET);
        button.click(ClickOptions::default()).await.unwrap();
        assert_eq!(presented_frame_waits(&harness), [Some("S1".to_owned())]);
        let seen = harness.control.commands_seen();
        let position =
            |matches: &dyn Fn(&SentCommand) -> bool| seen.iter().position(matches).unwrap();
        let scrolled = position(&|command| command.method == "DOM.scrollIntoViewIfNeeded");
        let waited = position(&|command| command.params["expression"] == PRESENTED_FRAME);
        let moved = position(&|command| command.method == "Input.dispatchMouseEvent");
        assert!(
            scrolled < waited && waited < moved,
            "{scrolled} {waited} {moved}"
        );
    }

    #[tokio::test]
    async fn main_frame_pointer_input_without_oopifs_does_not_wait_for_a_frame() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(scene::reply);
        let button = element(&harness, "T1", TARGET);
        button.click(ClickOptions::default()).await.unwrap();
        assert!(presented_frame_waits(&harness).is_empty());
        assert_eq!(mouse_events(&harness).len(), 3);
    }

    #[tokio::test]
    async fn a_page_without_animation_frames_delays_oopif_input_by_the_bound_only() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(|command| {
            if command.params["expression"] == PRESENTED_FRAME {
                return None;
            }
            scene::reply(command)
        });
        let button = element(&harness, "F2", TARGET);
        let started = tokio::time::Instant::now();
        let outcome = button.click(ClickOptions::default()).await.unwrap();
        assert_eq!(outcome, ActionOutcome::Completed);
        let elapsed = started.elapsed();
        assert!(
            elapsed >= PRESENTED_FRAME_WAIT && elapsed < PRESENTED_FRAME_WAIT * 4,
            "{elapsed:?}"
        );
        assert_eq!(mouse_events(&harness).len(), 3);
    }

    #[tokio::test]
    async fn nested_oopifs_sum_every_owner_content_origin() {
        let harness = scene::page_with_nested_oopif().await;
        let leaf = element(&harness, "F3", TARGET);
        let point = leaf
            .run(OpSpec::INPUT, |attempt| {
                let leaf = leaf.clone();
                async move { leaf.click_point_in(&attempt).await }
            })
            .await
            .unwrap()
            .read()
            .unwrap();
        assert_eq!(point.session.id().map(SessionId::as_str), Some("S3"));
        assert_eq!(point.local, (30.0, 60.0));
        assert_eq!(point.top, (81.0, 375.5));
        let chain: Vec<_> = point
            .boundaries
            .iter()
            .map(|boundary| {
                (
                    boundary.session.id().map(SessionId::to_string),
                    boundary.owner_backend_node_id,
                    boundary.point,
                )
            })
            .collect();
        assert_eq!(
            chain,
            vec![
                (Some("S2".to_owned()), NESTED_OWNER, (30.0, 78.0)),
                (Some("S1".to_owned()), OOPIF_OWNER, (81.0, 375.5)),
            ]
        );
        leaf.click(ClickOptions::default()).await.unwrap();
        assert!(
            mouse_events(&harness)
                .iter()
                .all(|(session, _, x, y)| session == "S1" && (*x, *y) == (81.0, 375.5))
        );
    }

    #[tokio::test]
    async fn main_frame_elements_click_at_their_local_centre() {
        let harness = scene::page_with_oopif().await;
        let button = element(&harness, "T1", TARGET);
        button.click(ClickOptions::default()).await.unwrap();
        assert_eq!(mouse_events(&harness)[0], at("S1", "moved", (120.0, 210.0)));
        assert!(sent_on(&harness, "DOM.getFrameOwner").is_empty());
    }

    #[tokio::test]
    async fn modified_double_clicks_carry_modifiers_buttons_and_counts() {
        let harness = scene::page_with_oopif().await;
        let button = element(&harness, "T1", TARGET);
        let options = ClickOptions {
            button: MouseButton::Right,
            modifiers: Modifiers::CTRL,
            click_count: 2,
        };
        button.click(options).await.unwrap();
        let events = input_events(&harness.control);
        let summary: Vec<(String, Value, Value, Value)> = events
            .iter()
            .map(|command| {
                let params = &command.params;
                (
                    params["type"].as_str().unwrap_or_default().to_owned(),
                    params.get("buttons").cloned().unwrap_or(Value::Null),
                    params.get("clickCount").cloned().unwrap_or(Value::Null),
                    params["modifiers"].clone(),
                )
            })
            .collect();
        let row = |kind: &str, buttons: Value, count: Value, modifiers: i64| {
            (kind.to_owned(), buttons, count, json!(modifiers))
        };
        assert_eq!(
            summary,
            vec![
                row("rawKeyDown", Value::Null, Value::Null, 2),
                row("mouseMoved", json!(0), json!(0), 2),
                row("mousePressed", json!(2), json!(1), 2),
                row("mouseReleased", json!(0), json!(1), 2),
                row("mousePressed", json!(2), json!(2), 2),
                row("mouseReleased", json!(0), json!(2), 2),
                row("keyUp", Value::Null, Value::Null, 0),
            ]
        );
        assert!(
            events
                .iter()
                .all(|command| command.session.as_deref() == Some("S1"))
        );
        assert_eq!(events[2].params["button"], "right");
    }

    #[tokio::test]
    async fn element_click_dispatches_on_the_owning_session_after_the_hit_chain() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(|command| {
            (declaration(command) == CLICK_TARGET_PROBE)
                .then(|| value(json!({"tag": "button", "type": ""})))
                .flatten()
                .or_else(|| scene::reply(command))
        });
        let button = element(&harness, "F2", TARGET);
        assert_eq!(
            button.element_click().await.unwrap(),
            ActionOutcome::Completed
        );
        let local = (64.25, 60.5);
        assert_eq!(
            mouse_events(&harness),
            vec![
                at("S2", "moved", local),
                at("S2", "pressed", local),
                at("S2", "released", local)
            ]
        );
        let hits = sent_on(&harness, "DOM.getNodeForLocation");
        let probes: Vec<_> = hits
            .iter()
            .map(|command| {
                (
                    command.session.clone().unwrap_or_default(),
                    command.params["x"].clone(),
                    command.params["y"].clone(),
                    command.params["includeUserAgentShadowDOM"].clone(),
                )
            })
            .collect();
        assert_eq!(
            probes,
            vec![
                ("S2".to_owned(), json!(64), json!(61), json!(true)),
                ("S1".to_owned(), json!(115), json!(358), json!(true)),
            ]
        );
    }

    fn overlay_scene(hit_on_page: i64) -> impl Fn(&SentCommand) -> Option<Value> + Send + Sync {
        move |command| {
            if declaration(command) == CLICK_TARGET_PROBE {
                return value(json!({"tag": "button", "type": ""}));
            }
            match (command.method.as_str(), command.session.as_deref()) {
                (HIT_TEST, Some("S1")) => {
                    Some(json!({"backendNodeId": hit_on_page, "frameId": "T1"}))
                }
                ("DOM.describeNode", _) => Some(json!({"node": {"backendNodeId": hit_on_page,
                    "nodeName": "DIV", "localName": "div",
                    "attributes": ["id", "overlay", "class", "cover  dim"]}})),
                _ => scene::reply(command),
            }
        }
    }

    const HEADER: i64 = 99;

    fn scroll_offset(session: &str) -> (i64, i64) {
        match session {
            "S1" => (10, 1000),
            "S2" => (5, 250),
            _ => (0, 0),
        }
    }

    fn hit_probes(harness: &PageHarness) -> Vec<(String, Value, Value)> {
        sent_on(harness, HIT_TEST)
            .iter()
            .map(|command| {
                (
                    command.session.clone().unwrap_or_default(),
                    command.params["x"].clone(),
                    command.params["y"].clone(),
                )
            })
            .collect()
    }

    /// Blink hit-tests `DOM.getNodeForLocation` in document coordinates of the local root of the
    /// session, so each scene node sits at its viewport point plus the scroll offset of that
    /// session and a sticky header answers every other point.
    fn scrolled_scene(command: &SentCommand) -> Option<Value> {
        let session = command.session.as_deref().unwrap_or_default();
        let (left, top) = scroll_offset(session);
        if declaration(command) == CLICK_TARGET_PROBE {
            return value(json!({"tag": "button", "type": ""}));
        }
        if declaration(command) == HIT_CHECK {
            return value(json!(false));
        }
        match command.method.as_str() {
            "Page.getLayoutMetrics" => {
                let mut metrics = scene::reply(command)?;
                metrics["cssLayoutViewport"]["pageX"] = json!(left);
                metrics["cssLayoutViewport"]["pageY"] = json!(top);
                Some(metrics)
            }
            HIT_TEST => {
                let x = command.params["x"].as_i64()? - left;
                let y = command.params["y"].as_i64()? - top;
                let (node, frame) = match (session, x, y) {
                    ("S1", 120, 210) => (TARGET, "T1"),
                    ("S1", 115, 358) => (OOPIF_OWNER, "T1"),
                    ("S2", 64, 61) => (TARGET, "F2"),
                    ("S2", _, _) => (HEADER, "F2"),
                    _ => (HEADER, "T1"),
                };
                Some(json!({"backendNodeId": node, "frameId": frame}))
            }
            "DOM.describeNode" => Some(json!({"node": {"backendNodeId": HEADER,
                "nodeName": "HEADER", "localName": "header", "attributes": ["id", "top"]}})),
            _ => scene::reply(command),
        }
    }

    #[tokio::test]
    async fn element_click_hit_tests_a_scrolled_main_frame_in_document_coordinates() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(scrolled_scene);
        let button = element(&harness, "T1", TARGET);
        assert_eq!(
            button.element_click().await.unwrap(),
            ActionOutcome::Completed
        );
        assert_eq!(
            hit_probes(&harness),
            vec![("S1".to_owned(), json!(130), json!(1210))]
        );
        let local = (120.0, 210.0);
        assert_eq!(
            mouse_events(&harness),
            vec![
                at("S1", "moved", local),
                at("S1", "pressed", local),
                at("S1", "released", local)
            ]
        );
    }

    #[tokio::test]
    async fn element_click_hit_tests_each_scrolled_local_root_in_its_own_document_coordinates() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(scrolled_scene);
        let button = element(&harness, "F2", TARGET);
        assert_eq!(
            button.element_click().await.unwrap(),
            ActionOutcome::Completed
        );
        assert_eq!(
            hit_probes(&harness),
            vec![
                ("S2".to_owned(), json!(69), json!(311)),
                ("S1".to_owned(), json!(125), json!(1358)),
            ]
        );
        let local = (64.25, 60.5);
        assert_eq!(
            mouse_events(&harness),
            vec![
                at("S2", "moved", local),
                at("S2", "pressed", local),
                at("S2", "released", local)
            ]
        );
    }

    #[tokio::test]
    async fn an_intercepted_click_on_a_scrolled_page_names_the_viewport_point() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(scrolled_scene);
        let covered = element(&harness, "T1", SOURCE);
        let error = covered.element_click().await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "element click intercepted: Element <button> is not clickable at point (120, 210). Other element would receive the click: <header#top>"
        );
        assert!(input_events(&harness.control).is_empty());
    }

    #[tokio::test]
    async fn an_overlay_over_the_iframe_intercepts_element_click() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(overlay_scene(99));
        let button = element(&harness, "F2", TARGET);
        let error = button.element_click().await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "element click intercepted: Element <button> is not clickable at point (115, 358). Other element would receive the click: <div#overlay.cover.dim>"
        );
        assert!(input_events(&harness.control).is_empty());
    }

    #[tokio::test]
    async fn the_w3c_click_ignores_an_overlay_like_chromedriver_actions() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(overlay_scene(99));
        let button = element(&harness, "F2", TARGET);
        assert_eq!(
            button.click(ClickOptions::default()).await.unwrap(),
            ActionOutcome::Completed
        );
        assert!(sent_on(&harness, "DOM.getNodeForLocation").is_empty());
    }

    #[tokio::test]
    async fn a_hit_inside_the_element_passes_the_local_check() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(|command| {
            if declaration(command) == CLICK_TARGET_PROBE {
                return value(json!({"tag": "a", "type": ""}));
            }
            if declaration(command) == HIT_CHECK {
                let passes = command.params["objectId"] == format!("obj-{TARGET}")
                    && command.params["arguments"][0]["objectId"] == "obj-43";
                return value(json!(passes));
            }
            match (command.method.as_str(), command.session.as_deref()) {
                (HIT_TEST, Some("S2")) => Some(json!({"backendNodeId": 43, "frameId": "F2"})),
                _ => scene::reply(command),
            }
        });
        let link = element(&harness, "F2", TARGET);
        assert_eq!(
            link.element_click().await.unwrap(),
            ActionOutcome::Completed
        );
        let resolved = sent_on(&harness, "DOM.resolveNode");
        assert!(
            resolved
                .iter()
                .any(|command| command.params["backendNodeId"] == 43
                    && command.params["executionContextId"] == 2)
        );
    }

    #[tokio::test]
    async fn element_click_refuses_file_inputs() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(|command| {
            (declaration(command) == CLICK_TARGET_PROBE)
                .then(|| value(json!({"tag": "input", "type": "file"})))
                .flatten()
                .or_else(|| scene::reply(command))
        });
        let upload = element(&harness, "T1", TARGET);
        assert!(matches!(
            upload.element_click().await,
            Err(BrowserError::InvalidArgument { message })
                if message == "element click on a file input is not allowed; use Upload File"
        ));
        assert!(input_events(&harness.control).is_empty());
    }

    #[tokio::test]
    async fn element_click_toggles_options_of_a_multiple_select_with_the_click_atom() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(|command| {
            if declaration(command) == CLICK_TARGET_PROBE {
                return value(json!({"tag": "option", "type": ""}));
            }
            if declaration(command) == OPTION_TOGGLEABLE.as_str() {
                return value(json!(true));
            }
            if calls_atom(command, "isSelected") {
                return value(json!(true));
            }
            if calls_atom(command, "click") {
                return value(Value::Null);
            }
            scene::reply(command)
        });
        let option = element(&harness, "T1", TARGET);
        assert_eq!(
            option.element_click().await.unwrap(),
            ActionOutcome::Completed
        );
        let click = harness
            .control
            .commands_seen()
            .into_iter()
            .find(|command| calls_atom(command, "click"))
            .unwrap();
        assert_eq!(click.params["arguments"], json!([{"value": false}]));
        assert!(input_events(&harness.control).is_empty());
    }

    #[tokio::test]
    async fn element_click_selects_a_single_select_option_with_atoms_installed_first() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(|command| {
            if declaration(command) == CLICK_TARGET_PROBE {
                return value(json!({"tag": "option", "type": ""}));
            }
            if declaration(command) == OPTION_TOGGLEABLE.as_str() {
                return value(json!(false));
            }
            if calls_atom(command, "isSelected") || calls_atom(command, "click") {
                return value(json!(true));
            }
            scene::reply(command)
        });
        let option = element(&harness, "T1", TARGET);
        assert_eq!(
            option.element_click().await.unwrap(),
            ActionOutcome::Completed
        );
        let seen = harness.control.commands_seen();
        let installs: Vec<usize> = seen
            .iter()
            .enumerate()
            .filter(|(_, command)| {
                command.method == "Runtime.evaluate" && command.params.get("contextId").is_some()
            })
            .map(|(index, _)| index)
            .collect();
        let selected = seen
            .iter()
            .position(|command| calls_atom(command, "isSelected"))
            .unwrap();
        let clicked = seen
            .iter()
            .position(|command| calls_atom(command, "click"))
            .unwrap();
        assert_eq!(installs.len(), 1);
        assert!(installs[0] < selected && selected < clicked);
        assert_eq!(seen[clicked].params["arguments"], json!([{"value": true}]));
    }

    #[tokio::test]
    async fn drag_between_two_frames_moves_the_pressed_button_on_the_page_session() {
        let harness = scene::page_with_oopif().await;
        let source = element(&harness, "T1", TARGET);
        let target = element(&harness, "F2", TARGET);
        assert_eq!(
            source.drag_to(&target).await.unwrap(),
            ActionOutcome::Completed
        );
        let to = (115.25, 358.0);
        assert_eq!(
            mouse_events(&harness),
            vec![
                at("S1", "moved", (120.0, 210.0)),
                at("S1", "pressed", (120.0, 210.0)),
                at("S1", "moved", to),
                at("S1", "released", to),
            ]
        );
        let drag_move = &input_events(&harness.control)[2];
        assert_eq!(drag_move.params["buttons"], 1);
        assert_eq!(drag_move.params["button"], "left");
        let measured: Vec<_> = sent_on(&harness, QUADS)
            .iter()
            .map(|command| command.session.clone().unwrap_or_default())
            .collect();
        assert_eq!(
            measured,
            ["S2", "S1", "S2"],
            "target, source, then the target again"
        );
        let scrolled: Vec<_> = sent_on(&harness, "DOM.scrollIntoViewIfNeeded")
            .iter()
            .map(|command| command.session.clone().unwrap_or_default())
            .collect();
        assert_eq!(
            scrolled,
            ["S2", "S1"],
            "the second target measure does not scroll"
        );
    }

    const SOURCE: i64 = 43;
    const SOURCE_QUAD: [f64; 8] = [100.0, 100.0, 140.0, 100.0, 140.0, 120.0, 100.0, 120.0];
    const SOURCE_CENTRE: (f64, f64) = (120.0, 110.0);

    async fn source_scroll_moves_the_target(
        moved: fn(&SentCommand) -> Option<Value>,
    ) -> PageHarness {
        let harness = scene::page_with_oopif().await;
        let scrolled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        harness.control.set_auto_reply(move |command| {
            let source = command.params["backendNodeId"] == SOURCE;
            match command.method.as_str() {
                "DOM.scrollIntoViewIfNeeded" if source => {
                    scrolled.store(true, std::sync::atomic::Ordering::SeqCst);
                    scene::reply(command)
                }
                QUADS if source => Some(json!({"quads": [SOURCE_QUAD]})),
                _ if scrolled.load(std::sync::atomic::Ordering::SeqCst) => {
                    moved(command).or_else(|| scene::reply(command))
                }
                _ => scene::reply(command),
            }
        });
        harness
    }

    fn target_quad(command: &SentCommand, quad: Value) -> Option<Value> {
        (command.method == QUADS && command.params["backendNodeId"] == TARGET)
            .then(|| json!({"quads": [quad]}))
    }

    fn drag_events(to: (f64, f64)) -> Vec<(String, String, f64, f64)> {
        vec![
            at("S1", "moved", SOURCE_CENTRE),
            at("S1", "pressed", SOURCE_CENTRE),
            at("S1", "moved", to),
            at("S1", "released", to),
        ]
    }

    #[tokio::test]
    async fn the_drop_point_is_measured_again_after_the_source_scroll() {
        let harness = source_scroll_moves_the_target(|command| {
            target_quad(command, json!([100, 450, 140, 450, 140, 470, 100, 470]))
        })
        .await;
        let source = element(&harness, "T1", SOURCE);
        let target = element(&harness, "T1", TARGET);
        assert_eq!(
            source.drag_to(&target).await.unwrap(),
            ActionOutcome::Completed
        );
        assert_eq!(mouse_events(&harness), drag_events((120.0, 460.0)));
        let scrolled: Vec<_> = sent_on(&harness, "DOM.scrollIntoViewIfNeeded")
            .iter()
            .map(|command| command.params["backendNodeId"].clone())
            .collect();
        assert_eq!(scrolled, [json!(TARGET), json!(SOURCE)]);
    }

    #[tokio::test]
    async fn the_drop_point_follows_an_iframe_moved_by_the_source_scroll() {
        let harness = source_scroll_moves_the_target(|command| {
            (command.method == "DOM.getBoxModel" && command.params["backendNodeId"] == OOPIF_OWNER)
                .then(|| json!({"model": {"content": [51, 500, 351, 500, 351, 650, 51, 650]}}))
        })
        .await;
        let source = element(&harness, "T1", SOURCE);
        let target = element(&harness, "F2", TARGET);
        assert_eq!(
            source.drag_to(&target).await.unwrap(),
            ActionOutcome::Completed
        );
        assert_eq!(
            mouse_events(&harness),
            drag_events((64.25 + 51.0, 60.5 + 500.0))
        );
    }

    #[tokio::test]
    async fn a_target_scrolled_out_by_the_source_fails_before_any_input() {
        let harness = source_scroll_moves_the_target(|command| {
            target_quad(command, json!([100, -300, 140, -300, 140, -280, 100, -280]))
        })
        .await;
        let source = element(&harness, "T1", SOURCE);
        let target = element(&harness, "T1", TARGET);
        let error = source.drag_to(&target).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "element not interactable: the drag source and target do not fit in the viewport together"
        );
        assert!(input_events(&harness.control).is_empty());
    }

    #[tokio::test]
    async fn a_drag_target_without_layout_fails_before_any_input() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(|command| {
            match (command.method.as_str(), command.session.as_deref()) {
                (QUADS, Some("S2")) => Some(json!({"quads": []})),
                _ => scene::reply(command),
            }
        });
        let source = element(&harness, "T1", TARGET);
        let target = element(&harness, "F2", TARGET);
        let error = source.drag_to(&target).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "element not interactable: element has no size or is not displayed"
        );
        assert!(input_events(&harness.control).is_empty());
    }

    fn labelled(
        harness: &PageHarness,
        frame: &str,
        backend_node_id: i64,
        reference: Option<&str>,
    ) -> Element {
        let frame = harness.page.frame(&FrameId::from(frame)).unwrap();
        let stamp = frame.stamp().unwrap();
        Element::new(frame, &stamp, backend_node_id, reference.map(Into::into))
    }

    async fn stale_drag(
        source: Option<&str>,
        target: Option<&str>,
        disconnected: &'static str,
    ) -> BrowserError {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(move |command| {
            (declaration(command) == scene::IS_CONNECTED
                && command.session.as_deref() == Some(disconnected))
            .then(|| value(json!(false)))
            .flatten()
            .or_else(|| scene::reply(command))
        });
        let source = labelled(&harness, "T1", TARGET, source);
        let target = labelled(&harness, "F2", TARGET, target);
        let error = source.drag_to(&target).await.unwrap_err();
        assert!(input_events(&harness.control).is_empty(), "{error}");
        error
    }

    #[tokio::test]
    async fn a_stale_drag_target_is_reported_under_its_own_ref() {
        let error = stale_drag(Some("e3"), Some("e9"), "S2").await;
        assert_eq!(
            error.to_string(),
            "Stale element ref 'e9' — take a new browser snapshot"
        );
        let error = stale_drag(Some("e3"), None, "S2").await;
        assert!(matches!(error, BrowserError::StaleElement), "{error}");
        let error = stale_drag(Some("e3"), Some("e9"), "S1").await;
        assert_eq!(
            error.to_string(),
            "Stale element ref 'e3' — take a new browser snapshot"
        );
        let error = stale_drag(None, Some("e9"), "S1").await;
        assert!(matches!(error, BrowserError::StaleElement), "{error}");
    }

    #[tokio::test]
    async fn a_drop_target_gone_after_the_source_scroll_is_reported_under_its_own_ref() {
        let harness = scene::page_with_oopif().await;
        let scrolled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        harness.control.set_auto_reply(move |command| {
            let source = command.params["backendNodeId"] == SOURCE;
            match command.method.as_str() {
                "DOM.scrollIntoViewIfNeeded" if source => {
                    scrolled.store(true, std::sync::atomic::Ordering::SeqCst);
                    scene::reply(command)
                }
                QUADS if source => Some(json!({"quads": [SOURCE_QUAD]})),
                QUADS if scrolled.load(std::sync::atomic::Ordering::SeqCst) => None,
                _ => scene::reply(command),
            }
        });
        let source = labelled(&harness, "T1", SOURCE, Some("e3"));
        let target = labelled(&harness, "T1", TARGET, Some("e9"));
        let mut control = harness.control;
        let drag = tokio::spawn(async move { source.drag_to(&target).await });
        let measure = control.wait_for(QUADS, Some("S1")).await;
        assert_eq!(measure.params["backendNodeId"], TARGET);
        control.reply_error(&measure, -32000, "No node with given id found");
        let error = drag.await.unwrap().unwrap_err();
        assert_eq!(
            error.to_string(),
            "Stale element ref 'e9' — take a new browser snapshot"
        );
        assert!(input_events(&control).is_empty());
    }

    #[tokio::test]
    async fn a_dialog_during_the_drag_releases_the_button_at_the_drop_point() {
        let mut harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(|command| {
            (command.params["buttons"] != 1 || command.params["type"] != "mouseMoved")
                .then(|| scene::reply(command))
                .flatten()
        });
        let source = element(&harness, "T1", TARGET);
        let target = element(&harness, "F2", TARGET);
        let drag = tokio::spawn(async move { source.drag_to(&target).await });
        let held_move = harness
            .control
            .wait_for("Input.dispatchMouseEvent", Some("S1"))
            .await;
        assert_eq!(mouse(&held_move), ("mouseMoved", 115.25, 358.0));
        open_dialog(&harness);
        assert!(matches!(
            drag.await.unwrap().unwrap(),
            ActionOutcome::DialogOpened { .. }
        ));
        harness.emit(
            "Page.javascriptDialogClosed",
            json!({"result": true, "userInput": ""}),
        );
        let released = wait_seen(&harness.control, "the pending release", |command| {
            command.params["type"] == "mouseReleased"
        })
        .await;
        assert_eq!(mouse(&released), ("mouseReleased", 115.25, 358.0));
    }

    const INNER_OWNER: i64 = 71;

    async fn with_same_process_child(harness: &PageHarness) {
        harness.emit_on(
            "S2",
            "Page.frameAttached",
            json!({"frameId": "F4", "parentFrameId": "F2"}),
        );
        harness.emit_on(
            "S2",
            "Page.frameNavigated",
            json!({"frame": {"id": "F4", "parentId": "F2", "loaderId": "L4", "url": "http://127.0.0.1/inner"}, "type": "Navigation"}),
        );
        harness.context_created("S2", "F4", 3, "S2-F4-main", false);
        harness.context_created("S2", "F4", 4, "S2-F4-util", true);
        let (frame, owner) = (FrameId::from("F4"), SessionId::from("S2"));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        harness
            .page
            .wait_for_state(deadline, |state| {
                state.contexts.util(&frame, &owner).map(drop)
            })
            .await
            .expect("the same-process child frame F4 is known with its util context");
    }

    fn inner_frame_scene(
        hit: i64,
        frame: &'static str,
        contains: bool,
    ) -> impl Fn(&SentCommand) -> Option<Value> + Send + Sync {
        move |command| {
            if declaration(command) == CLICK_TARGET_PROBE {
                return value(json!({"tag": "div", "type": ""}));
            }
            if declaration(command) == HIT_CHECK {
                return value(json!(contains));
            }
            match (command.method.as_str(), command.session.as_deref()) {
                (HIT_TEST, Some("S2")) => Some(json!({"backendNodeId": hit, "frameId": frame})),
                ("DOM.getFrameOwner", Some("S2")) if command.params["frameId"] == "F4" => {
                    Some(json!({"backendNodeId": INNER_OWNER}))
                }
                ("DOM.describeNode", _) => Some(json!({"node": {"backendNodeId": hit,
                    "nodeName": "DIV", "localName": "div", "attributes": ["id", "cover"]}})),
                _ => scene::reply(command),
            }
        }
    }

    fn resolved_nodes(harness: &PageHarness) -> Vec<Value> {
        sent_on(harness, "DOM.resolveNode")
            .iter()
            .map(|command| command.params["backendNodeId"].clone())
            .collect()
    }

    #[tokio::test]
    async fn a_hit_in_a_same_process_child_frame_counts_as_its_owner_iframe() {
        let harness = scene::page_with_oopif().await;
        with_same_process_child(&harness).await;
        harness
            .control
            .set_auto_reply(inner_frame_scene(70, "F4", true));
        let container = element(&harness, "F2", TARGET);
        assert_eq!(
            container.element_click().await.unwrap(),
            ActionOutcome::Completed
        );
        let owners = sent_on(&harness, "DOM.getFrameOwner");
        assert!(
            owners
                .iter()
                .any(|command| command.session.as_deref() == Some("S2")
                    && command.params["frameId"] == "F4")
        );
        let resolved = resolved_nodes(&harness);
        assert!(resolved.contains(&json!(INNER_OWNER)), "{resolved:?}");
        assert!(!resolved.contains(&json!(70)), "{resolved:?}");
        assert_eq!(mouse_events(&harness)[0].0, "S2");
    }

    #[tokio::test]
    async fn an_overlay_in_the_parent_document_of_a_same_process_frame_intercepts() {
        let harness = scene::page_with_oopif().await;
        with_same_process_child(&harness).await;
        harness
            .control
            .set_auto_reply(inner_frame_scene(80, "F2", true));
        let inner = element(&harness, "F4", TARGET);
        let error = inner.element_click().await.unwrap_err();
        assert!(
            matches!(&error, BrowserError::ClickIntercepted { message }
                if message.ends_with("Other element would receive the click: <div#cover>")),
            "{error}"
        );
        assert!(!resolved_nodes(&harness).contains(&json!(80)));
        assert!(input_events(&harness.control).is_empty());
    }

    #[tokio::test]
    async fn hover_moves_the_mouse_on_the_page_session_only() {
        let harness = scene::page_with_oopif().await;
        let menu = element(&harness, "F2", TARGET);
        assert_eq!(menu.hover().await.unwrap(), ActionOutcome::Completed);
        assert_eq!(
            mouse_events(&harness),
            vec![at("S1", "moved", (115.25, 358.0))]
        );
    }

    fn withhold_press(command: &SentCommand) -> Option<Value> {
        (command.params["type"] != "mousePressed")
            .then(|| scene::reply(command))
            .flatten()
    }

    fn open_dialog(harness: &PageHarness) {
        harness.emit(
            "Page.javascriptDialogOpening",
            json!({"url": "http://127.0.0.1/", "message": "sure?", "type": "confirm", "defaultPrompt": ""}),
        );
    }

    async fn click_into_a_dialog(harness: &mut PageHarness) -> ActionOutcome {
        harness.control.set_auto_reply(withhold_press);
        let button = element(harness, "F2", TARGET);
        let click = tokio::spawn(async move { button.click(ClickOptions::default()).await });
        let pressed = harness
            .control
            .wait_for("Input.dispatchMouseEvent", Some("S1"))
            .await;
        assert_eq!(pressed.params["type"], "mousePressed");
        open_dialog(harness);
        click.await.unwrap().unwrap()
    }

    fn releases(harness: &PageHarness) -> usize {
        input_events(&harness.control)
            .iter()
            .filter(|command| command.params["type"] == "mouseReleased")
            .count()
    }

    #[tokio::test]
    async fn a_dialog_during_mouse_pressed_releases_the_button_when_it_closes() {
        let mut harness = scene::page_with_oopif().await;
        let outcome = click_into_a_dialog(&mut harness).await;
        assert_eq!(
            outcome,
            ActionOutcome::DialogOpened {
                kind: crate::types::DialogType::Confirm,
                message: "sure?".to_owned()
            }
        );
        assert_eq!(
            releases(&harness),
            0,
            "the renderer is blocked by the dialog"
        );
        assert!(
            harness
                .page
                .inner
                .lock_state()
                .input
                .pending_release
                .is_some()
        );
        harness.emit(
            "Page.javascriptDialogClosed",
            json!({"result": true, "userInput": ""}),
        );
        let released = wait_seen(&harness.control, "the pending release", |command| {
            command.params["type"] == "mouseReleased"
        })
        .await;
        assert_eq!(released.session.as_deref(), Some("S1"));
        assert_eq!(mouse(&released), ("mouseReleased", 115.25, 358.0));
        assert_eq!(released.params["clickCount"], 1);
    }

    #[tokio::test]
    async fn a_dialog_handler_releases_the_button_once() {
        let mut harness = scene::page_with_oopif().await;
        click_into_a_dialog(&mut harness).await;
        crate::input::release_pending(&harness.page).unwrap();
        wait_seen(&harness.control, "the pending release", |command| {
            command.params["type"] == "mouseReleased"
        })
        .await;
        harness.emit(
            "Page.javascriptDialogClosed",
            json!({"result": false, "userInput": ""}),
        );
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        harness
            .page
            .wait_for_state(deadline, |state| {
                state.dialog.open().is_none().then_some(())
            })
            .await
            .unwrap();
        assert_eq!(releases(&harness), 1);
    }

    fn keys_of(harness: &PageHarness) -> Vec<(String, String)> {
        input_events(&harness.control)
            .iter()
            .filter(|command| command.method == "Input.dispatchKeyEvent")
            .map(|command| {
                (
                    command.params["type"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    command.params["key"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn a_failure_mid_click_still_releases_ctrl_and_the_button() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(withhold_press);
        let mut control = harness.control;
        let button = element_on(&harness.page, "T1");
        let options = ClickOptions {
            modifiers: Modifiers::CTRL,
            ..ClickOptions::default()
        };
        let click = tokio::spawn(async move { button.click(options).await });
        let pressed = control
            .wait_for("Input.dispatchMouseEvent", Some("S1"))
            .await;
        control.reply_error(&pressed, -32000, "Internal error");
        assert!(matches!(
            click.await.unwrap(),
            Err(BrowserError::Protocol { code: -32000, .. })
        ));
        wait_seen(&control, "the Ctrl keyUp", |command| {
            command.params["type"] == "keyUp"
        })
        .await;
        let released = wait_seen(&control, "the button release", |command| {
            command.params["type"] == "mouseReleased"
        })
        .await;
        assert_eq!(released.params["button"], "left");
    }

    fn element_on(page: &Page, frame: &str) -> Element {
        let frame = page.frame(&FrameId::from(frame)).unwrap();
        let stamp = frame.stamp().unwrap();
        Element::new(frame, &stamp, TARGET, None)
    }

    #[tokio::test]
    async fn a_dropped_click_future_still_releases_ctrl_and_the_button() {
        let mut harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(withhold_press);
        let button = element(&harness, "T1", TARGET);
        let options = ClickOptions {
            modifiers: Modifiers::CTRL,
            ..ClickOptions::default()
        };
        let click = tokio::spawn(async move { button.click(options).await });
        harness
            .control
            .wait_for("Input.dispatchMouseEvent", Some("S1"))
            .await;
        click.abort();
        assert!(click.await.unwrap_err().is_cancelled());
        wait_seen(&harness.control, "the Ctrl keyUp", |command| {
            command.params["type"] == "keyUp"
        })
        .await;
        wait_seen(&harness.control, "the button release", |command| {
            command.params["type"] == "mouseReleased"
        })
        .await;
        assert_eq!(
            keys_of(&harness),
            vec![
                ("rawKeyDown".to_owned(), "Control".to_owned()),
                ("keyUp".to_owned(), "Control".to_owned())
            ]
        );
    }

    #[tokio::test]
    async fn a_target_closed_after_commit_is_completed() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(|command| {
            (command.params["type"] != "mouseReleased")
                .then(|| scene::reply(command))
                .flatten()
        });
        let mut control = harness.control;
        let button = element_on(&harness.page, "F2");
        let click = tokio::spawn(async move { button.click(ClickOptions::default()).await });
        let released = control
            .wait_for("Input.dispatchMouseEvent", Some("S1"))
            .await;
        control.reply_error(&released, -32001, "Session with given id not found.");
        assert_eq!(click.await.unwrap().unwrap(), ActionOutcome::Completed);
    }

    async fn click_point_error(respond: fn(&SentCommand) -> Option<Value>) -> BrowserError {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(respond);
        let button = element(&harness, "F2", TARGET);
        let error = button.click(ClickOptions::default()).await.unwrap_err();
        assert!(input_events(&harness.control).is_empty(), "{error}");
        error
    }

    #[tokio::test]
    async fn click_point_errors_name_the_reason() {
        let error = scroll_failure("Node is detached from document").await;
        assert!(matches!(error, BrowserError::StaleElement), "{error}");
        let error = scroll_failure("Node does not have a layout object").await;
        assert_eq!(
            error.to_string(),
            "element not interactable: element has no size or is not displayed"
        );
        let error = click_point_error(|command| match command.method.as_str() {
            "DOM.getContentQuads" => Some(json!({"quads": []})),
            _ => scene::reply(command),
        })
        .await;
        assert_eq!(
            error.to_string(),
            "element not interactable: element has no size or is not displayed"
        );
        let error = click_point_error(|command| match command.method.as_str() {
            "DOM.getContentQuads" => {
                Some(json!({"quads": [[400, 200, 440, 200, 440, 220, 400, 220]]}))
            }
            _ => scene::reply(command),
        })
        .await;
        assert_eq!(
            error.to_string(),
            "element not interactable: element is outside the viewport"
        );
        let error = click_point_error(|command| {
            match (command.method.as_str(), command.session.as_deref()) {
                ("Page.getLayoutMetrics", Some("S1")) => Some(json!({"cssLayoutViewport":
                    {"pageX": 0, "pageY": 0, "clientWidth": 100, "clientHeight": 100}})),
                _ => scene::reply(command),
            }
        })
        .await;
        assert_eq!(
            error.to_string(),
            "element not interactable: element is outside the viewport"
        );
    }

    async fn scroll_failure(message: &str) -> BrowserError {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(|command| {
            (command.method != "DOM.scrollIntoViewIfNeeded")
                .then(|| scene::reply(command))
                .flatten()
        });
        let mut control = harness.control;
        let button = element_on(&harness.page, "F2");
        let click = tokio::spawn(async move { button.click(ClickOptions::default()).await });
        let scroll = control
            .wait_for("DOM.scrollIntoViewIfNeeded", Some("S2"))
            .await;
        control.reply_error(&scroll, -32000, message);
        click.await.unwrap().unwrap_err()
    }

    #[tokio::test]
    async fn a_new_document_during_the_click_point_sends_no_input() {
        let harness = scene::page_with_oopif().await;
        harness.control.set_auto_reply(|command| {
            (command.method != "DOM.getContentQuads")
                .then(|| scene::reply(command))
                .flatten()
        });
        let mut control = harness.control;
        let button = element_on(&harness.page, "T1");
        let page = harness.page.clone();
        let click = tokio::spawn(async move { button.click(ClickOptions::default()).await });
        let quads = control.wait_for("DOM.getContentQuads", Some("S1")).await;
        control.emit(
            "Page.frameNavigated",
            Some("S1"),
            json!({"frame": {"id": "T1", "loaderId": "L9", "url": "http://127.0.0.1/next"}, "type": "Navigation"}),
        );
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        page.wait_for_state(deadline, |state| {
            (state.frames.committed_loader(&FrameId::from("T1")) == Some(LoaderId::from("L9")))
                .then_some(())
        })
        .await
        .unwrap();
        control.reply(
            &quads,
            json!({"quads": [[100, 200, 140, 200, 140, 220, 100, 220]]}),
        );
        let error = click.await.unwrap().unwrap_err();
        assert!(
            matches!(error, BrowserError::StaleElement),
            "run_op retries LoaderChanged, and the retry finds the element stale: {error}"
        );
        assert!(input_events(&control).is_empty());
    }
}
