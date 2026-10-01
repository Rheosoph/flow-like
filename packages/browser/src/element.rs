// Derived from Chromium chrome/test/chromedriver @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE.
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize as _;
use serde_json::{Value, json};

use crate::error::{BrowserError, ErrorClass};
use crate::frames::{ContextRef, UTIL_WORLD};
use crate::page::{Frame, FrameStamp, Page, PageState};
use crate::script::{ELEMENT_KEY, World};
use crate::session::Session;
use crate::settle::{OpAttempt, OpOutcome, OpSpec};
use crate::types::{ExceptionDetails, FrameId, LoaderId, TargetId};

const CONTEXT_WAIT: Duration = Duration::from_millis(500);
const IS_CONNECTED: &str = "function(){return this.isConnected}";

pub(crate) const TEXT_CONTROL_TYPES: &[&str] = &["text", "search", "tel", "url", "password"];

pub(crate) const INPUT_CONTROL_TYPES: &[&str] = &[
    "text",
    "search",
    "url",
    "tel",
    "email",
    "password",
    "date",
    "month",
    "week",
    "time",
    "datetime-local",
    "number",
    "range",
    "color",
    "file",
];

pub(crate) const BOOLEAN_ATTRIBUTES: &[&str] = &[
    "allowfullscreen",
    "allowpaymentrequest",
    "allowusermedia",
    "async",
    "autofocus",
    "autoplay",
    "checked",
    "compact",
    "complete",
    "controls",
    "declare",
    "default",
    "defaultchecked",
    "defaultselected",
    "defer",
    "disabled",
    "ended",
    "formnovalidate",
    "hidden",
    "indeterminate",
    "iscontenteditable",
    "ismap",
    "itemscope",
    "loop",
    "multiple",
    "muted",
    "nohref",
    "nomodule",
    "noresize",
    "noshade",
    "novalidate",
    "nowrap",
    "open",
    "paused",
    "playsinline",
    "pubdate",
    "readonly",
    "required",
    "reversed",
    "scoped",
    "seamless",
    "seeking",
    "selected",
    "truespeed",
    "typemustmatch",
    "willvalidate",
];

const ATOMS: [(Atom, &str, &str); 9] = [
    (
        Atom::GetText,
        "getText",
        include_str!("js/atoms/get_text.js"),
    ),
    (
        Atom::IsDisplayed,
        "isDisplayed",
        include_str!("js/atoms/is_displayed.js"),
    ),
    (Atom::Clear, "clear", include_str!("js/atoms/clear.js")),
    (
        Atom::GetLocation,
        "getLocation",
        include_str!("js/atoms/get_location.js"),
    ),
    (
        Atom::GetSize,
        "getSize",
        include_str!("js/atoms/get_size.js"),
    ),
    (
        Atom::GetLocationInView,
        "getLocationInView",
        include_str!("js/atoms/get_location_in_view.js"),
    ),
    (
        Atom::IsSelected,
        "isSelected",
        include_str!("js/atoms/is_selected.js"),
    ),
    (
        Atom::IsEnabled,
        "isEnabled",
        include_str!("js/atoms/is_enabled.js"),
    ),
    (Atom::Click, "click", include_str!("js/atoms/click.js")),
];

static ATOM_INSTALLER: LazyLock<String> = LazyLock::new(|| {
    let members: Vec<String> = ATOMS
        .iter()
        .map(|(_, name, source)| format!("{name}:{source}"))
        .collect();
    format!(
        "(function(){{globalThis.__flowlike={{{}}}}})()",
        members.join(",")
    )
});

fn atom_call(atom: Atom) -> String {
    let (_, name, _) = ATOMS[atom as usize];
    format!("function(a,b,c){{return globalThis.__flowlike.{name}(this,a,b,c)}}")
}

const IS_USER_EDITABLE: &str = "function(inputTypes){var tag=this.tagName.toLowerCase();\
var control=tag===\"input\"&&inputTypes.indexOf(String(this.type).toLowerCase())>=0;\
var text=tag===\"textarea\";var contentEditable=!control&&!text&&this.isContentEditable===true;\
var readOnly=!contentEditable&&this.readOnly===true;\
return(control||text||contentEditable)&&!readOnly&&this.disabled!==true}";
const GET_ATTRIBUTE: &str = "function(name,boolean){if(boolean)return this.hasAttribute(name)?\"true\":null;\
return this.getAttribute(name)}";
const GET_PROPERTY: &str = "function(name){return this[name]}";
const TAG_NAME: &str = "function(){return this.tagName.toLowerCase()}";
const IS_MULTIPLE: &str = "function(){return this.multiple===true}";
const SCROLL_INTO_VIEW: &str =
    "function(){this.scrollIntoView({block:\"center\",inline:\"center\"})}";
const FOCUS: &str = "function(){this.focus()}";
const VIEWPORT: &str = "function(){return {x:window.pageXOffset,y:window.pageYOffset,\
width:document.documentElement.clientWidth,height:document.documentElement.clientHeight}}";
const IS_OPTION_TOGGLEABLE: &str = concat!(
    "function(){",
    include_str!("js/is_option_element_toggleable.js"),
    "\n;return isOptionElementToggleable(this)}"
);
const GET_ELEMENT_REGION: &str = concat!(
    "function(){",
    include_str!("js/get_element_region.js"),
    "\n;return getElementRegion(this)}"
);
const NOT_DISPLAYED: &str = "element has no size or is not displayed";
const VERBATIM_PREFIX: &str = "\\\\?\\";
const VERBATIM_UNC_PREFIX: &str = "\\\\?\\UNC\\";

#[derive(Clone)]
pub struct Element {
    pub(crate) frame: Frame,
    pub(crate) loader_id: LoaderId,
    pub(crate) local_root: TargetId,
    pub(crate) backend_node_id: i64,
    pub(crate) reference: Option<std::sync::Arc<str>>,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ElementRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Atom {
    GetText,
    IsDisplayed,
    Clear,
    GetLocation,
    GetSize,
    GetLocationInView,
    IsSelected,
    IsEnabled,
    Click,
}

pub(crate) struct ResolvedNode {
    pub session: crate::session::Session,
    pub object_id: String,
    pub context: crate::frames::ContextRef,
}

impl ResolvedNode {
    async fn call(&self, declaration: &str, args: Vec<Value>) -> crate::Result<Value> {
        call_on(&self.session, &self.object_id, declaration, args, true).await
    }

    async fn call_atom(&self, atom: Atom, args: Vec<Value>) -> crate::Result<Value> {
        self.call(&atom_call(atom), args).await
    }
}

enum OptionQuery {
    Css(String),
    XPath(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Viewport {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn stale_if_gone(error: BrowserError) -> BrowserError {
    if error.class() == ErrorClass::NodeGone {
        BrowserError::StaleElement
    } else {
        error
    }
}

pub(crate) fn javascript_error(details: &Value) -> BrowserError {
    let details = ExceptionDetails::deserialize(details).unwrap_or_default();
    let message = details
        .exception
        .and_then(|exception| exception.description)
        .unwrap_or(details.text);
    BrowserError::Javascript { message }
}

pub(crate) fn evaluation_result(mut reply: Value, return_by_value: bool) -> crate::Result<Value> {
    if let Some(details) = reply.get("exceptionDetails") {
        return Err(javascript_error(details));
    }
    let result = reply
        .get_mut("result")
        .map(Value::take)
        .unwrap_or(Value::Null);
    if !return_by_value {
        return Ok(result);
    }
    Ok(match result {
        Value::Object(mut object) => object.remove("value").unwrap_or(Value::Null),
        _ => Value::Null,
    })
}

async fn call_on(
    session: &Session,
    object_id: &str,
    declaration: &str,
    args: Vec<Value>,
    return_by_value: bool,
) -> crate::Result<Value> {
    let arguments: Vec<Value> = args
        .into_iter()
        .map(|value| json!({"value": value}))
        .collect();
    let params = json!({
        "objectId": object_id,
        "functionDeclaration": declaration,
        "arguments": arguments,
        "returnByValue": return_by_value,
        "awaitPromise": false,
    });
    let reply = session.send("Runtime.callFunctionOn", params).await?;
    evaluation_result(reply, return_by_value)
}

pub(crate) async fn world_context(
    frame: &Frame,
    session: &Session,
    world: World,
    stamp: &FrameStamp,
) -> crate::Result<ContextRef> {
    let id = &frame.id;
    let owner = &stamp.session;
    let lookup = |state: &PageState| match world {
        World::Main => state.contexts.main(id, owner),
        World::Util => state.contexts.util(id, owner),
    };
    if let Some(context) = lookup(&frame.page.inner.lock_state()) {
        return Ok(context);
    }
    if world == World::Util {
        let params = json!({"frameId": id, "worldName": UTIL_WORLD, "grantUniveralAccess": true});
        session.send("Page.createIsolatedWorld", params).await?;
    }
    let missing = match world {
        World::Main => "Cannot find default execution context",
        World::Util => "Cannot find context with specified id",
    };
    let deadline = tokio::time::Instant::now() + CONTEXT_WAIT;
    frame
        .page
        .wait_for_state(deadline, lookup)
        .await
        .ok_or_else(|| {
            BrowserError::protocol("DOM.resolveNode", -32000, format!("{missing} (frame {id})"))
        })
}

fn unexpected(what: &str, expected: &str, value: &Value) -> BrowserError {
    BrowserError::protocol(
        "Runtime.callFunctionOn",
        -32000,
        format!("{what} returned {value} instead of {expected}"),
    )
}

fn string_result(value: Value, what: &str) -> crate::Result<String> {
    match value {
        Value::String(text) => Ok(text),
        other => Err(unexpected(what, "a string", &other)),
    }
}

fn bool_result(value: &Value, what: &str) -> crate::Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| unexpected(what, "a boolean", value))
}

fn number(value: &Value, key: &str, what: &str) -> crate::Result<f64> {
    value[key]
        .as_f64()
        .ok_or_else(|| unexpected(what, &format!("a number in \"{key}\""), value))
}

fn element_rect(location: &Value, size: &Value) -> crate::Result<ElementRect> {
    Ok(ElementRect {
        x: number(location, "x", "GET_LOCATION")?,
        y: number(location, "y", "GET_LOCATION")?,
        width: number(size, "width", "GET_SIZE")?,
        height: number(size, "height", "GET_SIZE")?,
    })
}

fn error_summary(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    match line.split_once(": ") {
        Some((name, rest))
            if name.ends_with("Error") && name.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            rest.to_owned()
        }
        _ => line.to_owned(),
    }
}

fn css_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' | '\\' => {
                quoted.push('\\');
                quoted.push(character);
            }
            '\0' => quoted.push('\u{FFFD}'),
            control if control.is_control() => {
                quoted.push_str(&format!("\\{:x} ", u32::from(control)));
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn xpath_literal(value: &str) -> String {
    if !value.contains('"') {
        return format!("\"{value}\"");
    }
    if !value.contains('\'') {
        return format!("'{value}'");
    }
    let parts: Vec<String> = value.split('"').map(|part| format!("\"{part}\"")).collect();
    format!("concat({})", parts.join(", '\"', "))
}

fn without_verbatim_prefix(path: &str) -> String {
    if let Some(share) = path.strip_prefix(VERBATIM_UNC_PREFIX) {
        return format!("\\\\{share}");
    }
    match path.strip_prefix(VERBATIM_PREFIX) {
        Some(local) if local.as_bytes().get(1) == Some(&b':') => local.to_owned(),
        _ => path.to_owned(),
    }
}

async fn canonical_files(paths: &[PathBuf]) -> crate::Result<Vec<String>> {
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let canonical = tokio::fs::canonicalize(path).await.map_err(|error| {
            tracing::debug!(%error, "upload path rejected");
            BrowserError::InvalidArgument {
                message: format!("File not found: {}", path.display()),
            }
        })?;
        files.push(without_verbatim_prefix(&canonical.to_string_lossy()));
    }
    Ok(files)
}

fn truncated_region(region: &Value) -> crate::Result<Value> {
    let side =
        |key: &str| number(region, key, "getElementRegion").map(|value| value.trunc() as i64);
    Ok(json!({
        "left": side("left")?,
        "top": side("top")?,
        "width": side("width")?,
        "height": side("height")?,
    }))
}

fn quad_bounds(reply: &Value) -> Option<Bounds> {
    let points: Vec<(f64, f64)> = reply["quads"]
        .as_array()?
        .iter()
        .filter_map(Value::as_array)
        .flat_map(|quad| quad.as_chunks::<2>().0)
        .filter_map(|[x, y]| Some((x.as_f64()?, y.as_f64()?)))
        .collect();
    let (first, rest) = points.split_first()?;
    let start = Bounds {
        left: first.0,
        top: first.1,
        right: first.0,
        bottom: first.1,
    };
    let bounds = rest.iter().fold(start, |bounds, (x, y)| Bounds {
        left: bounds.left.min(*x),
        top: bounds.top.min(*y),
        right: bounds.right.max(*x),
        bottom: bounds.bottom.max(*y),
    });
    (bounds.right > bounds.left && bounds.bottom > bounds.top).then_some(bounds)
}

fn layout_error(error: BrowserError) -> BrowserError {
    match &error {
        BrowserError::Protocol { message, .. } if message.contains("layout object") => {
            BrowserError::NotInteractable {
                message: NOT_DISPLAYED.to_owned(),
            }
        }
        _ => stale_if_gone(error),
    }
}

impl Bounds {
    fn shifted(self, (dx, dy): (f64, f64)) -> Bounds {
        Bounds {
            left: self.left + dx,
            top: self.top + dy,
            right: self.right + dx,
            bottom: self.bottom + dy,
        }
    }
}

impl Viewport {
    async fn of(page: &Page) -> crate::Result<Viewport> {
        let view = page.main_frame().evaluate_in(World::Util, VIEWPORT).await?;
        Ok(Viewport {
            x: number(&view, "x", "the viewport probe")?,
            y: number(&view, "y", "the viewport probe")?,
            width: number(&view, "width", "the viewport probe")?,
            height: number(&view, "height", "the viewport probe")?,
        })
    }

    fn element_clip(&self, location: &Value, size: &Value) -> crate::Result<Value> {
        let x = number(location, "x", "GET_LOCATION_IN_VIEW")?;
        let y = number(location, "y", "GET_LOCATION_IN_VIEW")?;
        let width = number(size, "width", "GET_SIZE")?;
        let height = number(size, "height", "GET_SIZE")?;
        Ok(json!({
            "x": x + self.x,
            "y": y + self.y,
            "width": (self.width - x).min(width),
            "height": (self.height - y).min(height),
            "scale": 1,
        }))
    }

    fn cropped_clip(&self, bounds: Bounds) -> crate::Result<Value> {
        let left = bounds.left.max(0.0);
        let top = bounds.top.max(0.0);
        let right = bounds.right.min(self.width);
        let bottom = bounds.bottom.min(self.height);
        if right <= left || bottom <= top {
            return Err(BrowserError::NotInteractable {
                message: "element is outside the viewport".to_owned(),
            });
        }
        Ok(json!({
            "x": left + self.x,
            "y": top + self.y,
            "width": right - left,
            "height": bottom - top,
            "scale": 1,
        }))
    }
}

impl Element {
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn loader_id(&self) -> &LoaderId {
        &self.loader_id
    }

    pub fn backend_node_id(&self) -> i64 {
        self.backend_node_id
    }

    pub fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }

    pub fn web_element_id(&self) -> String {
        format!(
            "f.{}.d.{}.e.{}",
            self.frame.id, self.loader_id, self.backend_node_id
        )
    }

    pub fn web_element_json(&self) -> serde_json::Value {
        json!({ ELEMENT_KEY: self.web_element_id() })
    }

    pub fn node_ref(&self) -> crate::refs::NodeRef {
        crate::refs::NodeRef {
            page: self.frame.page.target_id().clone(),
            local_root: self.local_root.clone(),
            frame_id: self.frame.id.clone(),
            loader_id: self.loader_id.clone(),
            backend_node_id: self.backend_node_id,
        }
    }

    pub async fn text(&self) -> crate::Result<String> {
        let text = self.read_atom(Atom::GetText, Vec::new()).await?;
        string_result(text, "GET_TEXT")
    }

    pub async fn is_displayed(&self) -> crate::Result<bool> {
        let shown = self
            .read_atom(Atom::IsDisplayed, vec![Value::Bool(false)])
            .await?;
        bool_result(&shown, "IS_DISPLAYED")
    }

    pub async fn is_enabled(&self) -> crate::Result<bool> {
        let enabled = self.read_atom(Atom::IsEnabled, Vec::new()).await?;
        bool_result(&enabled, "IS_ENABLED")
    }

    pub async fn is_selected(&self) -> crate::Result<bool> {
        let selected = self.read_atom(Atom::IsSelected, Vec::new()).await?;
        bool_result(&selected, "IS_SELECTED")
    }

    pub async fn rect(&self) -> crate::Result<ElementRect> {
        let (location, size) = self
            .run(OpSpec::READ, |attempt| async move {
                let location = self
                    .call_atom_in(&attempt, Atom::GetLocation, Vec::new())
                    .await?;
                let size = self
                    .call_atom_in(&attempt, Atom::GetSize, Vec::new())
                    .await?;
                Ok((location, size))
            })
            .await?
            .read()?;
        element_rect(&location, &size)
    }

    pub async fn attribute(&self, name: &str) -> crate::Result<Option<String>> {
        let boolean = BOOLEAN_ATTRIBUTES.contains(&name.to_ascii_lowercase().as_str());
        let args = vec![json!(name), json!(boolean)];
        match self.read_function(World::Util, GET_ATTRIBUTE, args).await? {
            Value::Null => Ok(None),
            value => string_result(value, "getAttribute").map(Some),
        }
    }

    pub async fn property(&self, name: &str) -> crate::Result<serde_json::Value> {
        self.read_function(World::Main, GET_PROPERTY, vec![json!(name)])
            .await
    }

    pub async fn tag_name(&self) -> crate::Result<String> {
        let tag = self
            .read_function(World::Util, TAG_NAME, Vec::new())
            .await?;
        string_result(tag, "tagName")
    }

    pub async fn outer_html(&self) -> crate::Result<String> {
        self.html("outerHTML").await
    }

    pub async fn inner_html(&self) -> crate::Result<String> {
        self.html("innerHTML").await
    }

    pub async fn clear(&self) -> crate::Result<()> {
        self.act(|attempt| async move { self.clear_in(&attempt).await })
            .await
    }

    pub async fn set_files(&self, paths: &[std::path::PathBuf]) -> crate::Result<()> {
        let files = canonical_files(paths).await?;
        self.act(|attempt| {
            let files = files.clone();
            async move { self.set_files_in(&attempt, files).await }
        })
        .await
    }

    pub async fn select_by_value(&self, value: &str) -> crate::Result<()> {
        let query = OptionQuery::Css(format!("option[value={}]", css_string(value)));
        let missing = format!("Could not find an option with value '{value}'");
        self.select_options(query, missing).await
    }

    pub async fn select_by_exact_text(&self, text: &str) -> crate::Result<()> {
        let query = OptionQuery::XPath(format!(".//option[text() = {}]", xpath_literal(text)));
        let missing = format!("Could not find an option with text '{text}'");
        self.select_options(query, missing).await
    }

    pub async fn scroll_into_view(&self) -> crate::Result<()> {
        self.perform(World::Main, SCROLL_INTO_VIEW).await
    }

    pub async fn focus(&self) -> crate::Result<()> {
        self.perform(World::Main, FOCUS).await
    }

    pub async fn screenshot_png(&self) -> crate::Result<Vec<u8>> {
        let data = self
            .run(OpSpec::READ, |attempt| async move {
                self.screenshot_in(&attempt).await
            })
            .await?
            .read()?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| {
                BrowserError::protocol(
                    "Page.captureScreenshot",
                    -32000,
                    format!("the screenshot is not valid base64: {error}"),
                )
            })
    }

    pub(crate) fn new(
        frame: Frame,
        stamp: &FrameStamp,
        backend_node_id: i64,
        reference: Option<std::sync::Arc<str>>,
    ) -> Element {
        Element {
            frame,
            loader_id: stamp.loader.clone(),
            local_root: stamp.local_root.clone(),
            backend_node_id,
            reference,
        }
    }

    pub(crate) async fn run<T, F, Fut>(&self, spec: OpSpec, op: F) -> crate::Result<OpOutcome<T>>
    where
        F: FnMut(OpAttempt) -> Fut,
        Fut: std::future::Future<Output = crate::Result<T>>,
    {
        let spec = OpSpec {
            frame_scoped: false,
            ..spec
        };
        self.frame
            .page
            .run_op(&self.frame.id, spec, op)
            .await
            .map_err(|error| self.map_stale(error))
    }

    pub(crate) fn map_stale(&self, error: crate::BrowserError) -> crate::BrowserError {
        match &self.reference {
            Some(reference) if error.class() == ErrorClass::NodeGone => BrowserError::StaleRef {
                reference: reference.to_string(),
            },
            _ => error,
        }
    }

    pub(crate) async fn resolve_in(
        &self,
        attempt: &OpAttempt,
        world: World,
    ) -> crate::Result<ResolvedNode> {
        let before = self.frame.stamp()?;
        if self.loader_id != before.loader || self.local_root != before.local_root {
            return Err(BrowserError::StaleElement);
        }
        let session = self
            .frame
            .page
            .inner
            .connection
            .session(Some(before.session.clone()));
        let context = world_context(&self.frame, &session, world, &before).await?;
        tracing::trace!(
            attempt = attempt.index,
            node = self.backend_node_id,
            "resolving node"
        );
        let group = attempt.objects.on(&session);
        let object_id = self.connected_object(&session, &context, group).await?;
        if self.frame.stamp()? != before {
            return Err(BrowserError::LoaderChanged {
                method: "DOM.resolveNode".to_owned(),
            });
        }
        Ok(ResolvedNode {
            session,
            object_id,
            context,
        })
    }

    async fn connected_object(
        &self,
        session: &Session,
        context: &ContextRef,
        group: &str,
    ) -> crate::Result<String> {
        let params = json!({
            "backendNodeId": self.backend_node_id,
            "executionContextId": context.id,
            "objectGroup": group,
        });
        let resolved = session
            .send("DOM.resolveNode", params)
            .await
            .map_err(stale_if_gone)?;
        let Some(object_id) = resolved["object"]["objectId"].as_str() else {
            let message = format!(
                "node {} resolved without an object id",
                self.backend_node_id
            );
            return Err(BrowserError::protocol("DOM.resolveNode", -32000, message));
        };
        let connected = call_on(session, object_id, IS_CONNECTED, Vec::new(), true)
            .await
            .map_err(stale_if_gone)?;
        if connected != Value::Bool(true) {
            return Err(BrowserError::StaleElement);
        }
        Ok(object_id.to_owned())
    }

    pub(crate) async fn call_function_in(
        &self,
        attempt: &OpAttempt,
        world: World,
        declaration: &str,
        args: Vec<serde_json::Value>,
        return_by_value: bool,
    ) -> crate::Result<serde_json::Value> {
        let node = self.resolve_in(attempt, world).await?;
        call_on(
            &node.session,
            &node.object_id,
            declaration,
            args,
            return_by_value,
        )
        .await
    }

    pub(crate) async fn call_atom_in(
        &self,
        attempt: &OpAttempt,
        atom: Atom,
        extra_args: Vec<serde_json::Value>,
    ) -> crate::Result<serde_json::Value> {
        let node = self.atom_node_in(attempt).await?;
        node.call_atom(atom, extra_args).await
    }

    async fn atom_node_in(&self, attempt: &OpAttempt) -> crate::Result<ResolvedNode> {
        let node = self.resolve_in(attempt, World::Util).await?;
        self.install_atoms(&node).await?;
        Ok(node)
    }

    pub(crate) fn ensure_document(&self, method: &str) -> crate::Result<()> {
        let stamp = self.frame.stamp()?;
        if stamp.loader == self.loader_id && stamp.local_root == self.local_root {
            return Ok(());
        }
        Err(BrowserError::LoaderChanged {
            method: method.to_owned(),
        })
    }

    async fn install_atoms(&self, node: &ResolvedNode) -> crate::Result<()> {
        let unique_id = &node.context.unique_id;
        if self
            .frame
            .page
            .inner
            .lock_state()
            .contexts
            .atoms_installed(unique_id)
        {
            return Ok(());
        }
        let params = json!({
            "expression": ATOM_INSTALLER.as_str(),
            "contextId": node.context.id,
            "returnByValue": true,
        });
        let reply = node.session.send("Runtime.evaluate", params).await?;
        evaluation_result(reply, true)?;
        self.frame
            .page
            .inner
            .lock_state()
            .contexts
            .mark_atoms_installed(unique_id);
        Ok(())
    }

    async fn read_atom(&self, atom: Atom, args: Vec<Value>) -> crate::Result<Value> {
        self.run(OpSpec::READ, |attempt| {
            let args = args.clone();
            async move { self.call_atom_in(&attempt, atom, args).await }
        })
        .await?
        .read()
    }

    async fn read_function(
        &self,
        world: World,
        declaration: &'static str,
        args: Vec<Value>,
    ) -> crate::Result<Value> {
        self.run(OpSpec::READ, |attempt| {
            let args = args.clone();
            async move {
                self.call_function_in(&attempt, world, declaration, args, true)
                    .await
            }
        })
        .await?
        .read()
    }

    async fn html(&self, property: &str) -> crate::Result<String> {
        let args = vec![json!(property)];
        match self.read_function(World::Main, GET_PROPERTY, args).await? {
            Value::Null => Ok(String::new()),
            value => string_result(value, property),
        }
    }

    async fn perform(&self, world: World, declaration: &'static str) -> crate::Result<()> {
        self.run(OpSpec::READ, |attempt| async move {
            self.call_function_in(&attempt, world, declaration, Vec::new(), true)
                .await
                .map(drop)
        })
        .await
        .map(drop)
    }

    async fn act<F, Fut>(&self, op: F) -> crate::Result<()>
    where
        F: FnMut(OpAttempt) -> Fut,
        Fut: std::future::Future<Output = crate::Result<()>>,
    {
        self.run(OpSpec::INPUT, op).await.map(drop)
    }

    /// Input ops race the dialog signal themselves once committed; run_op stops racing at commit.
    /// A dialog applied before `count` was read is only visible as the pending dialog.
    async fn commit<T>(
        &self,
        attempt: &OpAttempt,
        action: impl std::future::Future<Output = crate::Result<T>>,
    ) -> crate::Result<OpOutcome<T>> {
        let page = &self.frame.page;
        let count = page.dialog_count();
        if let Some(dialog) = page.pending_dialog() {
            return Ok(OpOutcome::DialogOpened(dialog));
        }
        attempt.commit();
        tokio::select! {
            result = action => result.map(OpOutcome::Done),
            dialog = page.dialog_opened_after(count) => Ok(OpOutcome::DialogOpened(dialog)),
        }
    }

    async fn clear_in(&self, attempt: &OpAttempt) -> crate::Result<()> {
        let node = self.atom_node_in(attempt).await?;
        ensure_clearable(&node).await?;
        let cleared = node.call_atom(Atom::Clear, Vec::new());
        match self.commit(attempt, cleared).await {
            Err(BrowserError::Javascript { message }) => Err(BrowserError::InvalidElementState {
                message: error_summary(&message),
            }),
            outcome => outcome.map(drop),
        }
    }

    async fn set_files_in(&self, attempt: &OpAttempt, files: Vec<String>) -> crate::Result<()> {
        let node = self.resolve_in(attempt, World::Util).await?;
        if files.len() > 1 {
            let multiple = node.call(IS_MULTIPLE, Vec::new()).await?;
            if !bool_result(&multiple, "multiple")? {
                return Err(BrowserError::InvalidArgument {
                    message: "the element can not hold multiple files".to_owned(),
                });
            }
        }
        let params = json!({"files": files, "backendNodeId": self.backend_node_id});
        let upload = async {
            node.session
                .send("DOM.setFileInputFiles", params)
                .await
                .map_err(stale_if_gone)?;
            self.ensure_document("DOM.setFileInputFiles")
        };
        self.commit(attempt, upload).await.map(drop)
    }

    async fn select_options(&self, query: OptionQuery, missing: String) -> crate::Result<()> {
        let (query, missing) = (&query, missing.as_str());
        self.act(|attempt| async move { self.select_in(&attempt, query, missing).await })
            .await
    }

    async fn select_in(
        &self,
        attempt: &OpAttempt,
        query: &OptionQuery,
        missing: &str,
    ) -> crate::Result<()> {
        let options = self.matching_options_in(attempt, query).await?;
        let Some(first) = options.first() else {
            return Err(BrowserError::NotFound {
                message: missing.to_owned(),
            });
        };
        let targets = if self.is_multiple_in(attempt).await? {
            options.as_slice()
        } else {
            std::slice::from_ref(first)
        };
        let page = &self.frame.page;
        let count = page.dialog_count();
        tokio::select! {
            selected = select_each_in(attempt, targets) => selected,
            _ = page.dialog_opened_after(count) => Ok(()),
        }
    }

    async fn is_multiple_in(&self, attempt: &OpAttempt) -> crate::Result<bool> {
        let multiple = self
            .call_function_in(attempt, World::Util, IS_MULTIPLE, Vec::new(), true)
            .await?;
        bool_result(&multiple, "multiple")
    }

    async fn matching_options_in(
        &self,
        attempt: &OpAttempt,
        query: &OptionQuery,
    ) -> crate::Result<Vec<Element>> {
        match query {
            OptionQuery::Css(css) => self.frame.find_css_in(attempt, css, Some(self)).await,
            OptionQuery::XPath(xpath) => self.frame.find_xpath_in(attempt, xpath, Some(self)).await,
        }
    }

    async fn select_option_in(&self, attempt: &OpAttempt) -> crate::Result<OpOutcome<()>> {
        let node = self.atom_node_in(attempt).await?;
        let selected = node.call_atom(Atom::IsSelected, Vec::new()).await?;
        if bool_result(&selected, "IS_SELECTED")? {
            return Ok(OpOutcome::Done(()));
        }
        Ok(match self.click_option_in(attempt, &node, false).await? {
            OpOutcome::Done(_) => OpOutcome::Done(()),
            OpOutcome::DialogOpened(dialog) => OpOutcome::DialogOpened(dialog),
        })
    }

    async fn click_option_in(
        &self,
        attempt: &OpAttempt,
        node: &ResolvedNode,
        selected: bool,
    ) -> crate::Result<OpOutcome<Value>> {
        let toggleable = node.call(IS_OPTION_TOGGLEABLE, Vec::new()).await?;
        let select = !bool_result(&toggleable, "isOptionElementToggleable")? || !selected;
        let click = node.call_atom(Atom::Click, vec![Value::Bool(select)]);
        self.commit(attempt, click).await
    }

    async fn screenshot_in(&self, attempt: &OpAttempt) -> crate::Result<String> {
        let page = &self.frame.page;
        page.activate_for_capture().await?;
        let clip = if self.frame.is_main() {
            self.document_clip_in(attempt).await?
        } else {
            self.frame_clip_in(attempt).await?
        };
        let params = json!({"format": "png", "clip": clip});
        let reply = page
            .session()
            .send("Page.captureScreenshot", params)
            .await?;
        match reply.get("data") {
            Some(Value::String(data)) => Ok(data.clone()),
            _ => Err(BrowserError::protocol(
                "Page.captureScreenshot",
                -32000,
                "the reply carries no image data",
            )),
        }
    }

    async fn document_clip_in(&self, attempt: &OpAttempt) -> crate::Result<Value> {
        let region = self
            .call_function_in(attempt, World::Util, GET_ELEMENT_REGION, Vec::new(), true)
            .await?;
        let in_view = vec![Value::Bool(false), truncated_region(&region)?];
        let location = self
            .call_atom_in(attempt, Atom::GetLocationInView, in_view)
            .await?;
        let size = self
            .call_atom_in(attempt, Atom::GetSize, Vec::new())
            .await?;
        Viewport::of(&self.frame.page)
            .await?
            .element_clip(&location, &size)
    }

    async fn frame_clip_in(&self, attempt: &OpAttempt) -> crate::Result<Value> {
        let node = self.resolve_in(attempt, World::Util).await?;
        let bounds = self.content_bounds(&node).await?;
        let origin = self.owner_origin().await?;
        self.ensure_document("DOM.getContentQuads")?;
        Viewport::of(&self.frame.page)
            .await?
            .cropped_clip(bounds.shifted(origin))
    }

    async fn content_bounds(&self, node: &ResolvedNode) -> crate::Result<Bounds> {
        let target = json!({"backendNodeId": self.backend_node_id});
        node.session
            .send("DOM.scrollIntoViewIfNeeded", target.clone())
            .await
            .map_err(layout_error)?;
        let quads = node
            .session
            .send("DOM.getContentQuads", target)
            .await
            .map_err(layout_error)?;
        quad_bounds(&quads).ok_or_else(|| BrowserError::NotInteractable {
            message: NOT_DISPLAYED.to_owned(),
        })
    }

    /// Sum of the owner iframe content-box origins across every out-of-process boundary up to the page.
    async fn owner_origin(&self) -> crate::Result<(f64, f64)> {
        let page = &self.frame.page;
        let main = page.main_frame().id;
        let mut origin = (0.0, 0.0);
        let mut current = self.frame.clone();
        loop {
            let root = FrameId::new(current.stamp()?.local_root.as_str());
            if root == main {
                return Ok(origin);
            }
            let parent = page
                .inner
                .lock_state()
                .frames
                .get(&root)
                .and_then(|node| node.parent.clone())
                .ok_or_else(|| BrowserError::FrameInTransit {
                    frame: root.to_string(),
                })?;
            let parent = Frame {
                page: page.clone(),
                id: parent,
            };
            let (x, y) = owner_content_origin(&parent.session()?, &root).await?;
            origin = (origin.0 + x, origin.1 + y);
            current = parent;
        }
    }
}

async fn ensure_clearable(node: &ResolvedNode) -> crate::Result<()> {
    let types = vec![json!(INPUT_CONTROL_TYPES)];
    let editable = node.call(IS_USER_EDITABLE, types).await?;
    if !bool_result(&editable, "the clear pre-check")? {
        return Err(BrowserError::InvalidElementState {
            message: "Element must be user-editable in order to clear it".to_owned(),
        });
    }
    let displayed = node
        .call_atom(Atom::IsDisplayed, vec![Value::Bool(true)])
        .await?;
    if !bool_result(&displayed, "IS_DISPLAYED")? {
        return Err(BrowserError::NotInteractable {
            message: "element is not displayed".to_owned(),
        });
    }
    Ok(())
}

async fn select_each_in(attempt: &OpAttempt, options: &[Element]) -> crate::Result<()> {
    for option in options {
        if let OpOutcome::DialogOpened(_) = option.select_option_in(attempt).await? {
            break;
        }
    }
    Ok(())
}

async fn owner_content_origin(session: &Session, frame: &FrameId) -> crate::Result<(f64, f64)> {
    let owner = session
        .send("DOM.getFrameOwner", json!({"frameId": frame}))
        .await?;
    let backend_node_id = owner["backendNodeId"].as_i64().ok_or_else(|| {
        BrowserError::protocol(
            "DOM.getFrameOwner",
            -32000,
            format!("no owner node for frame {frame}"),
        )
    })?;
    let model = session
        .send("DOM.getBoxModel", json!({"backendNodeId": backend_node_id}))
        .await?;
    let content = &model["model"]["content"];
    match (content[0].as_f64(), content[1].as_f64()) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(BrowserError::protocol(
            "DOM.getBoxModel",
            -32000,
            format!("the owner of frame {frame} has no content box"),
        )),
    }
}

pub(crate) fn mint_elements(
    frame: &Frame,
    before: &FrameStamp,
    backend_node_ids: Vec<i64>,
    method: &str,
) -> crate::Result<Vec<Element>> {
    if frame.stamp()? != *before {
        return Err(BrowserError::LoaderChanged {
            method: method.to_owned(),
        });
    }
    Ok(backend_node_ids
        .into_iter()
        .map(|backend_node_id| Element::new(frame.clone(), before, backend_node_id, None))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{PageHarness, default_auto_reply};
    use crate::transport::memory::SentCommand;
    use crate::types::LoaderId;

    fn connected_node(command: &SentCommand) -> Option<Value> {
        match command.method.as_str() {
            "DOM.resolveNode" => Some(json!({"object": {"type": "object", "objectId": "node-1"}})),
            "Runtime.callFunctionOn" if command.params["functionDeclaration"] == IS_CONNECTED => {
                Some(json!({"result": {"type": "boolean", "value": true}}))
            }
            _ => default_auto_reply(command),
        }
    }

    #[test]
    fn the_ported_control_lists_keep_their_upstream_sizes() {
        assert_eq!(TEXT_CONTROL_TYPES.len(), 5);
        assert_eq!(INPUT_CONTROL_TYPES.len(), 15);
        assert_eq!(BOOLEAN_ATTRIBUTES.len(), 46);
        assert!(
            TEXT_CONTROL_TYPES
                .iter()
                .all(|kind| INPUT_CONTROL_TYPES.contains(kind))
        );
    }

    fn element(harness: &PageHarness, reference: Option<&str>) -> Element {
        let frame = harness.page.main_frame();
        let stamp = frame.stamp().unwrap();
        Element::new(frame, &stamp, 42, reference.map(Into::into))
    }

    async fn resolve(element: &Element, world: World) -> crate::Result<String> {
        element
            .run(OpSpec::READ, |attempt| {
                let element = element.clone();
                async move { Ok(element.resolve_in(&attempt, world).await?.object_id) }
            })
            .await?
            .read()
    }

    #[test]
    fn the_atom_table_follows_the_enum_order() {
        for (index, (atom, _, source)) in ATOMS.iter().enumerate() {
            assert_eq!(*atom as usize, index);
            assert!(source.starts_with("/* Derived from Chromium third_party/selenium-atoms"));
        }
        assert_eq!(
            atom_call(Atom::Click),
            "function(a,b,c){return globalThis.__flowlike.click(this,a,b,c)}"
        );
    }

    #[tokio::test]
    async fn resolve_uses_the_owning_session_and_world_context() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(connected_node);
        let element = element(&harness, None);
        assert_eq!(resolve(&element, World::Util).await.unwrap(), "node-1");
        let resolved = harness
            .control
            .commands_seen()
            .into_iter()
            .find(|command| command.method == "DOM.resolveNode")
            .unwrap();
        assert_eq!(resolved.session.as_deref(), Some("S1"));
        assert_eq!(resolved.params["executionContextId"], 2);
        let group = resolved.params["objectGroup"].as_str().unwrap();
        assert!(group.starts_with("flowlike-op-"), "{group}");
        assert_eq!(element.web_element_id(), "f.T1.d.L1.e.42");
    }

    fn sent_groups(harness: &PageHarness, method: &str) -> Vec<(Option<String>, Value)> {
        harness
            .control
            .commands_seen()
            .into_iter()
            .filter(|command| command.method == method)
            .map(|command| (command.session, command.params["objectGroup"].clone()))
            .collect()
    }

    async fn released(harness: &PageHarness, group: &Value) -> Vec<(Option<String>, Value)> {
        harness
            .sent("the op object group release", |command| {
                command.method == "Runtime.releaseObjectGroup"
                    && command.params["objectGroup"] == *group
            })
            .await;
        sent_groups(harness, "Runtime.releaseObjectGroup")
    }

    fn resolved_group(harness: &PageHarness) -> Value {
        let resolved = sent_groups(harness, "DOM.resolveNode");
        let (_, group) = resolved.last().expect("the op resolved no node");
        group.clone()
    }

    fn withholding_is_connected(command: &SentCommand) -> Option<Value> {
        if command.params["functionDeclaration"] == IS_CONNECTED {
            return None;
        }
        connected_node(command)
    }

    #[tokio::test]
    async fn an_element_op_releases_its_own_object_group_once_when_it_ends() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(connected_node);
        let element = element(&harness, None);
        element
            .run(OpSpec::READ, |attempt| {
                let element = element.clone();
                async move {
                    element.resolve_in(&attempt, World::Util).await?;
                    element.resolve_in(&attempt, World::Main).await.map(drop)
                }
            })
            .await
            .unwrap()
            .read()
            .unwrap();
        let first = resolved_group(&harness);
        assert_eq!(
            sent_groups(&harness, "DOM.resolveNode"),
            vec![(Some("S1".to_owned()), first.clone()); 2]
        );
        assert_eq!(
            released(&harness, &first).await,
            [(Some("S1".to_owned()), first.clone())]
        );
        resolve(&element, World::Util).await.unwrap();
        let second = resolved_group(&harness);
        assert_ne!(first, second, "every op has its own group");
        assert_eq!(
            released(&harness, &second).await,
            [
                (Some("S1".to_owned()), first),
                (Some("S1".to_owned()), second)
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_element_op_releases_its_object_group() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(|command| {
            if command.params["functionDeclaration"] == IS_CONNECTED {
                return Some(json!({"result": {"type": "boolean", "value": false}}));
            }
            connected_node(command)
        });
        let element = element(&harness, None);
        assert!(matches!(
            resolve(&element, World::Util).await,
            Err(BrowserError::StaleElement)
        ));
        let group = resolved_group(&harness);
        assert_eq!(
            released(&harness, &group).await,
            [(Some("S1".to_owned()), group)]
        );
    }

    #[tokio::test]
    async fn a_canceled_element_op_releases_its_object_group() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(withholding_is_connected);
        let element = element(&harness, None);
        let op = tokio::spawn(async move { resolve(&element, World::Util).await });
        harness
            .sent("the connected check", |command| {
                command.params["functionDeclaration"] == IS_CONNECTED
            })
            .await;
        op.abort();
        assert!(op.await.unwrap_err().is_cancelled());
        let group = resolved_group(&harness);
        assert_eq!(
            released(&harness, &group).await,
            [(Some("S1".to_owned()), group)]
        );
    }

    #[tokio::test]
    async fn an_element_op_past_its_deadline_still_releases_its_object_group() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(withholding_is_connected);
        let element = element(&harness, None);
        let op = element.run(OpSpec::OUTPUT, |attempt| {
            let element = element.clone();
            async move { element.resolve_in(&attempt, World::Util).await.map(drop) }
        });
        let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        let op = crate::connection::with_op_deadline(deadline, op);
        let outcome = tokio::time::timeout(Duration::from_secs(5), op)
            .await
            .expect("the op never finished");
        assert!(
            matches!(&outcome, Err(BrowserError::Timeout { .. })),
            "{:?}",
            outcome.as_ref().err()
        );
        let group = resolved_group(&harness);
        assert_eq!(
            released(&harness, &group).await,
            [(Some("S1".to_owned()), group)]
        );
    }

    #[tokio::test]
    async fn a_stale_loader_fails_before_any_dom_command() {
        let harness = PageHarness::new().await;
        let mut element = element(&harness, None);
        element.loader_id = LoaderId::from("L0");
        assert!(matches!(
            resolve(&element, World::Main).await,
            Err(BrowserError::StaleElement)
        ));
        assert!(
            !harness
                .control
                .commands_seen()
                .iter()
                .any(|command| command.method.starts_with("DOM."))
        );
    }

    #[tokio::test]
    async fn referenced_elements_report_the_verbatim_stale_ref() {
        let harness = PageHarness::new().await;
        harness
            .control
            .set_auto_reply(|command| match command.method.as_str() {
                "DOM.resolveNode" => Some(json!({"object": {"objectId": "node-1"}})),
                "Runtime.callFunctionOn" => {
                    Some(json!({"result": {"type": "boolean", "value": false}}))
                }
                _ => default_auto_reply(command),
            });
        let element = element(&harness, Some("e12"));
        let error = resolve(&element, World::Util).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Stale element ref 'e12' — take a new browser snapshot"
        );
    }

    #[tokio::test]
    async fn atoms_are_installed_once_per_context() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(|command| {
            connected_node(command).or_else(|| match command.method.as_str() {
                "Runtime.evaluate" if command.params["contextId"] == 2 => {
                    Some(json!({"result": {"type": "undefined"}}))
                }
                "Runtime.callFunctionOn" => {
                    Some(json!({"result": {"type": "string", "value": "hello"}}))
                }
                _ => None,
            })
        });
        let element = element(&harness, None);
        for _ in 0..2 {
            let text = element
                .run(OpSpec::READ, |attempt| {
                    let element = element.clone();
                    async move {
                        element
                            .call_atom_in(&attempt, Atom::GetText, Vec::new())
                            .await
                    }
                })
                .await
                .unwrap()
                .read()
                .unwrap();
            assert_eq!(text, "hello");
        }
        let seen = harness.control.commands_seen();
        let installs: Vec<_> = seen
            .iter()
            .filter(|command| {
                command.method == "Runtime.evaluate" && command.params["contextId"] == 2
            })
            .collect();
        assert_eq!(installs.len(), 1);
        let expression = installs[0].params["expression"].as_str().unwrap();
        assert!(expression.starts_with("(function(){globalThis.__flowlike={getText:"));
        let calls = seen
            .iter()
            .filter(|command| {
                command.params["functionDeclaration"]
                    .as_str()
                    .is_some_and(|declaration| declaration.contains("__flowlike.getText(this"))
            })
            .count();
        assert_eq!(calls, 2);
    }

    #[tokio::test]
    async fn exceptions_become_javascript_errors() {
        let harness = PageHarness::new().await;
        harness.control.set_auto_reply(|command| {
            connected_node(command).or_else(|| {
                (command.method == "Runtime.callFunctionOn").then(|| {
                    json!({
                        "result": {"type": "object"},
                        "exceptionDetails": {"text": "Uncaught", "exception": {"type": "object", "description": "TypeError: nope"}}
                    })
                })
            })
        });
        let element = element(&harness, None);
        let outcome = element
            .run(OpSpec::READ, |attempt| {
                let element = element.clone();
                async move {
                    element
                        .call_function_in(
                            &attempt,
                            World::Main,
                            "function(){throw 1}",
                            Vec::new(),
                            true,
                        )
                        .await
                }
            })
            .await;
        assert!(
            matches!(outcome, Err(BrowserError::Javascript { message }) if message == "TypeError: nope")
        );
    }

    #[tokio::test]
    async fn minting_after_a_new_document_reports_loader_changed() {
        let harness = PageHarness::new().await;
        let frame = harness.page.main_frame();
        let before = frame.stamp().unwrap();
        assert_eq!(
            mint_elements(&frame, &before, vec![1, 2], "DOM.describeNode")
                .unwrap()
                .len(),
            2
        );
        harness.emit(
            "Page.frameNavigated",
            json!({"frame": {"id": "T1", "loaderId": "L2", "url": "http://127.0.0.1/next"}, "type": "Navigation"}),
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let navigated = harness
            .page
            .wait_for_state(deadline, |state| {
                (state.frames.committed_loader(frame.id()) == Some(LoaderId::from("L2")))
                    .then_some(())
            })
            .await;
        assert!(navigated.is_some(), "navigation never applied");
        assert!(matches!(
            mint_elements(&frame, &before, vec![1], "DOM.describeNode"),
            Err(BrowserError::LoaderChanged { method }) if method == "DOM.describeNode"
        ));
    }

    #[test]
    fn option_selectors_quote_every_value() {
        assert_eq!(css_string("b"), "\"b\"");
        assert_eq!(css_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(css_string("line\nbreak"), "\"line\\a break\"");
        assert_eq!(css_string("nul\0"), "\"nul\u{FFFD}\"");
        assert_eq!(xpath_literal("Two"), "\"Two\"");
        assert_eq!(xpath_literal("say \"hi\""), "'say \"hi\"'");
        assert_eq!(
            xpath_literal("it's \"x\""),
            "concat(\"it's \", '\"', \"x\", '\"', \"\")"
        );
    }

    #[test]
    fn canonical_windows_paths_lose_the_verbatim_prefix() {
        assert_eq!(
            without_verbatim_prefix(r"\\?\C:\data\a.txt"),
            r"C:\data\a.txt"
        );
        assert_eq!(
            without_verbatim_prefix(r"\\?\UNC\server\share\a.txt"),
            r"\\server\share\a.txt"
        );
        assert_eq!(
            without_verbatim_prefix(r"\\?\Volume{1}\a.txt"),
            r"\\?\Volume{1}\a.txt"
        );
        assert_eq!(without_verbatim_prefix("/tmp/a.txt"), "/tmp/a.txt");
    }

    #[test]
    fn atom_exceptions_keep_only_their_message() {
        assert_eq!(
            error_summary(
                "InvalidElementStateError: Element must be user-editable in order to clear it.\n    at x"
            ),
            "Element must be user-editable in order to clear it."
        );
        assert_eq!(error_summary("Error: nope"), "nope");
        assert_eq!(error_summary("Uncaught: odd"), "Uncaught: odd");
    }

    #[test]
    fn element_clips_follow_chromedriver_arithmetic() {
        let view = Viewport {
            x: 0.0,
            y: 100.0,
            width: 1185.0,
            height: 600.0,
        };
        let clip = view
            .element_clip(
                &json!({"x": 150.5, "y": 20.25}),
                &json!({"width": 87, "height": 47}),
            )
            .unwrap();
        assert_eq!(
            clip,
            json!({"x": 150.5, "y": 120.25, "width": 87.0, "height": 47.0, "scale": 1})
        );
        let wide = view
            .element_clip(
                &json!({"x": 1000, "y": 0}),
                &json!({"width": 600, "height": 60}),
            )
            .unwrap();
        assert_eq!(wide["width"], 185.0);
        assert_eq!(
            truncated_region(&json!({"left": 0.9, "top": -0.5, "width": 87.99, "height": 47}))
                .unwrap(),
            json!({"left": 0, "top": 0, "width": 87, "height": 47})
        );
    }

    #[test]
    fn frame_clips_are_cropped_to_the_top_level_viewport() {
        let view = Viewport {
            x: 10.0,
            y: 20.0,
            width: 800.0,
            height: 600.0,
        };
        let quads = json!({"quads": [[20, 50, 108.5, 50, 108.5, 70, 20, 70], [0, 70, 30, 70, 30, 80, 0, 80]]});
        let bounds = quad_bounds(&quads).unwrap();
        assert_eq!(
            bounds,
            Bounds {
                left: 0.0,
                top: 50.0,
                right: 108.5,
                bottom: 80.0
            }
        );
        let overhanging = Bounds {
            left: -10.0,
            top: 590.0,
            right: 40.0,
            bottom: 700.0,
        };
        assert_eq!(
            view.cropped_clip(overhanging).unwrap(),
            json!({"x": 10.0, "y": 610.0, "width": 40.0, "height": 10.0, "scale": 1})
        );
        let outside = Bounds {
            left: 900.0,
            top: 0.0,
            right: 950.0,
            bottom: 10.0,
        };
        assert!(matches!(
            view.cropped_clip(outside),
            Err(BrowserError::NotInteractable { .. })
        ));
        assert!(quad_bounds(&json!({"quads": []})).is_none());
        assert!(quad_bounds(&json!({"quads": [[5, 5, 5, 5, 5, 5, 5, 5]]})).is_none());
    }

    #[tokio::test]
    async fn public_ops_on_a_referenced_element_report_the_stale_ref() {
        let harness = PageHarness::new().await;
        let mut element = element(&harness, Some("e7"));
        element.loader_id = LoaderId::from("L0");
        let error = element.text().await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Stale element ref 'e7' — take a new browser snapshot"
        );
        assert!(
            !harness
                .control
                .commands_seen()
                .iter()
                .any(|command| command.method.starts_with("DOM."))
        );
    }
}
