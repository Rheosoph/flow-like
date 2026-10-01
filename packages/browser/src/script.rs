// Derived from Chromium chrome/test/chromedriver @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};

use crate::connection::{with_op_deadline, without_op_deadline};
use crate::element::{Element, evaluation_result, javascript_error, mint_elements, world_context};
use crate::error::BrowserError;
use crate::frames::ContextRef;
use crate::page::{Frame, FrameStamp};
use crate::session::Session;
use crate::settle::{OpAttempt, OpKind, OpSpec, SCRIPT_TIMEOUT};
use crate::types::{FrameId, LoaderId};

pub const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";
pub const SHADOW_KEY: &str = "shadow-6066-11e4-a52e-4f735466cecf";
pub const WINDOW_KEY: &str = "window-fcc6-11e5-b4f8-330a88ab9d7f";
const FRAME_KEY: &str = "frame-075b-4da1-b6ba-e579c2d3230a";

const CALL_METHOD: &str = "Runtime.callFunctionOn";
const INVALID_SELECTOR: &str = "invalid selector: ";

const CALL_WRAPPER: &str = concat!(
    "function(script,args,w3c){var nodes=[];",
    "for(var i=3;i<arguments.length;i++)nodes[i-3]=arguments[i];",
    "return (function() { ",
    include_str!("js/call_function.js"),
    "; return callFunction.apply(null, arguments) }\n).apply(null, [function() { ",
    include_str!("js/execute_script.js"),
    "; return executeScript.apply(null, arguments) }\n, [script, args], w3c, nodes]); }",
);
const FIND_CSS: &str = "function(selector){var root=this&&this.nodeType?this:document;var found;\
try{found=root.querySelectorAll(selector)}catch(error){throw \"invalid selector: \"+error.message}\
return Array.prototype.slice.call(found)}";
const FIND_XPATH: &str = "function(selector){var root=this&&this.nodeType?this:document;var result;\
try{result=document.evaluate(selector,root,null,XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,null)}\
catch(error){throw \"invalid selector: \"+error.message}var nodes=[];\
for(var i=0;i<result.snapshotLength;i++){var node=result.snapshotItem(i);\
if(node.nodeType!==1)throw \"invalid selector: The result of the xpath expression is not an element\";\
nodes.push(node)}return nodes}";
const PAGE_SOURCE: &str = "function(){return (document.documentElement||{}).outerHTML||\"\"}";

static NEXT_OBJECT_GROUP: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum World {
    Main,
    Util,
}

#[derive(Clone, Copy, Debug)]
pub struct ScriptOptions {
    pub world: World,
    pub idempotent: bool,
}

impl ScriptOptions {
    pub const USER: ScriptOptions = ScriptOptions {
        world: World::Main,
        idempotent: false,
    };
    pub const PROBE: ScriptOptions = ScriptOptions {
        world: World::Main,
        idempotent: true,
    };
    pub const INTERNAL: ScriptOptions = ScriptOptions {
        world: World::Util,
        idempotent: true,
    };
}

#[derive(Clone)]
pub enum ScriptArg {
    Json(serde_json::Value),
    Element(crate::element::Element),
}

#[derive(Clone)]
pub struct ScriptValue {
    json: serde_json::Value,
    elements: Vec<crate::element::Element>,
}

impl ScriptValue {
    fn null() -> ScriptValue {
        ScriptValue {
            json: Value::Null,
            elements: Vec::new(),
        }
    }

    pub fn json(&self) -> &serde_json::Value {
        &self.json
    }

    pub fn into_json(self) -> serde_json::Value {
        self.json
    }

    pub fn is_null(&self) -> bool {
        self.json.is_null()
    }

    pub fn element(&self) -> crate::Result<crate::element::Element> {
        let id = element_ref(&self.json).ok_or_else(|| not_elements(&self.json, "an element"))?;
        self.elements
            .iter()
            .find(|element| element.web_element_id() == id)
            .cloned()
            .ok_or_else(|| not_returned(id))
    }

    pub fn elements(&self) -> crate::Result<Vec<crate::element::Element>> {
        let expected = "an array of elements";
        let items = self
            .json
            .as_array()
            .ok_or_else(|| not_elements(&self.json, expected))?;
        let minted: HashMap<String, &Element> = self
            .elements
            .iter()
            .map(|element| (element.web_element_id(), element))
            .collect();
        items
            .iter()
            .map(|item| {
                let id = element_ref(item).ok_or_else(|| not_elements(&self.json, expected))?;
                minted
                    .get(id)
                    .map(|element| (*element).clone())
                    .ok_or_else(|| not_returned(id))
            })
            .collect()
    }
}

fn not_returned(id: &str) -> BrowserError {
    BrowserError::NotFound {
        message: format!("The script result references element {id}, which it did not return"),
    }
}

fn element_ref(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    object
        .get(ELEMENT_KEY)
        .or_else(|| object.get(SHADOW_KEY))?
        .as_str()
}

fn not_elements(value: &Value, expected: &str) -> BrowserError {
    let text = value.to_string();
    let preview = match text.char_indices().nth(200) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text,
    };
    BrowserError::NotFound {
        message: format!("The script result is not {expected}: {preview}"),
    }
}

struct WorldScope {
    stamp: FrameStamp,
    session: Session,
    context: ContextRef,
}

enum SearchRoot {
    Scope(String),
    Document(String),
}

impl SearchRoot {
    fn call_params(self, declaration: &str, selector: &str, group: &str) -> Value {
        let (key, target) = match self {
            SearchRoot::Scope(object) => ("objectId", object),
            SearchRoot::Document(context) => ("uniqueContextId", context),
        };
        let mut params = json!({
            "functionDeclaration": declaration,
            "arguments": [{"value": selector}],
            "objectGroup": group,
        });
        params[key] = Value::String(target);
        params
    }
}

/// A uniquely named object group, released on every session that used it when dropped
/// (also on error, cancellation and after the op deadline).
pub(crate) struct ObjectGroup {
    name: String,
    sessions: Mutex<Vec<Session>>,
}

impl ObjectGroup {
    pub(crate) fn new(kind: &str) -> ObjectGroup {
        let id = NEXT_OBJECT_GROUP.fetch_add(1, Ordering::Relaxed);
        ObjectGroup {
            name: format!("flowlike-{kind}-{id}"),
            sessions: Mutex::default(),
        }
    }

    /// The group name for objects created on `session`, which the drop then releases.
    pub(crate) fn on(&self, session: &Session) -> &str {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        if !sessions.iter().any(|known| known.id() == session.id()) {
            sessions.push(session.clone());
        }
        &self.name
    }
}

impl std::fmt::Debug for ObjectGroup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str(&self.name)
    }
}

impl Drop for ObjectGroup {
    fn drop(&mut self) {
        let sessions = self
            .sessions
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        for session in sessions.drain(..) {
            let params = json!({"objectGroup": self.name});
            let released =
                without_op_deadline(|| session.send_nowait("Runtime.releaseObjectGroup", params));
            if let Err(error) = released {
                tracing::debug!(group = %self.name, %error, "object group not released");
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum SerializedRef {
    Node {
        backend_node_id: i64,
        loader: Option<LoaderId>,
    },
    Window {
        context: String,
    },
    Unsupported,
}

impl SerializedRef {
    fn parse(entry: &Value) -> SerializedRef {
        let value = &entry["value"];
        match (entry["type"].as_str(), value["backendNodeId"].as_i64()) {
            (Some("node"), Some(backend_node_id)) => SerializedRef::Node {
                backend_node_id,
                loader: value["loaderId"].as_str().map(LoaderId::from),
            },
            (Some("window"), _) => {
                value["context"]
                    .as_str()
                    .map_or(SerializedRef::Unsupported, |context| {
                        SerializedRef::Window {
                            context: context.to_owned(),
                        }
                    })
            }
            _ => SerializedRef::Unsupported,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Link {
    Node(String),
    Window { key: &'static str, context: String },
    Unsupported,
}

fn malformed(detail: &str) -> BrowserError {
    BrowserError::protocol(CALL_METHOD, -32000, format!("the script result {detail}"))
}

fn parse_response(text: &str) -> crate::Result<Value> {
    serde_json::from_str(text).or_else(|error| {
        crate::protocol::sanitize_lone_surrogates(text)
            .and_then(|repaired| serde_json::from_str(&repaired).ok())
            .ok_or_else(|| malformed(&format!("is not valid JSON ({error})")))
    })
}

fn response_value(response: &Value) -> crate::Result<Value> {
    match response["status"].as_i64() {
        Some(0) => Ok(response.get("value").cloned().unwrap_or(Value::Null)),
        Some(10) => Err(BrowserError::StaleElement),
        Some(_) => Err(BrowserError::Javascript {
            message: match &response["value"] {
                Value::String(message) => message.clone(),
                other => other.to_string(),
            },
        }),
        None => Err(malformed("has no status")),
    }
}

/// Deep serialization writes a node that occurs twice only once; later occurrences carry just the
/// shared `weakLocalObjectReference`.
fn resolve_weak_references(entries: &[Value]) -> Vec<Value> {
    let has_value = |entry: &Value| entry.get("value").is_some_and(|value| !value.is_null());
    let known: HashMap<i64, &Value> = entries
        .iter()
        .filter(|entry| has_value(entry))
        .filter_map(|entry| Some((entry["weakLocalObjectReference"].as_i64()?, entry)))
        .collect();
    entries
        .iter()
        .map(|entry| {
            let shared = entry["weakLocalObjectReference"].as_i64();
            match shared.and_then(|reference| known.get(&reference)) {
                Some(original) if !has_value(entry) => (*original).clone(),
                _ => entry.clone(),
            }
        })
        .collect()
}

fn parse_wrapper_result(result: &Value) -> crate::Result<(Value, Vec<SerializedRef>)> {
    let list = result["deepSerializedValue"]["value"]
        .as_array()
        .filter(|list| !list.is_empty())
        .ok_or_else(|| malformed("carries no serialized list"))?;
    let text = list[0]["value"]
        .as_str()
        .ok_or_else(|| malformed("list does not start with the JSON response"))?;
    let value = response_value(&parse_response(text)?)?;
    let refs = resolve_weak_references(&list[1..])
        .iter()
        .map(SerializedRef::parse)
        .collect();
    Ok((value, refs))
}

fn link_value(value: &mut Value, links: &[Link]) -> crate::Result<()> {
    match value {
        Value::Array(items) => items
            .iter_mut()
            .try_for_each(|item| link_value(item, links)),
        Value::Object(object) => {
            let key = [ELEMENT_KEY, SHADOW_KEY, WINDOW_KEY]
                .into_iter()
                .find(|key| object.contains_key(*key));
            let Some(key) = key else {
                return object
                    .values_mut()
                    .try_for_each(|item| link_value(item, links));
            };
            let index = object[key].as_u64().unwrap_or(u64::MAX);
            let link = usize::try_from(index)
                .ok()
                .and_then(|index| links.get(index))
                .ok_or_else(|| {
                    malformed(&format!(
                        "references node {}, which it does not contain",
                        object[key]
                    ))
                })?;
            match (key == WINDOW_KEY, link) {
                (false, Link::Node(id)) => {
                    object.insert(key.to_owned(), Value::String(id.clone()));
                }
                (true, Link::Window { key, context }) => {
                    object.remove(WINDOW_KEY);
                    object.insert((*key).to_owned(), Value::String(context.clone()));
                }
                _ => return Err(malformed("references a node of the wrong kind")),
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn selector_error(details: &Value) -> BrowserError {
    match details["exception"]["value"].as_str() {
        Some(message) if message.starts_with(INVALID_SELECTOR) => BrowserError::InvalidArgument {
            message: message.to_owned(),
        },
        _ => javascript_error(details),
    }
}

fn script_spec(args: &[ScriptArg], options: ScriptOptions) -> OpSpec {
    let base = if options.idempotent {
        OpSpec::FRAME_READ
    } else {
        OpSpec::SCRIPT
    };
    let has_element = args.iter().any(|arg| matches!(arg, ScriptArg::Element(_)));
    OpSpec {
        frame_scoped: base.frame_scoped && !has_element,
        ..base
    }
}

pub(crate) fn array_items(properties: &Value) -> Vec<&str> {
    let mut items: Vec<(usize, &str)> = properties["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|property| {
            let index = property["name"].as_str()?.parse().ok()?;
            Some((index, property["value"]["objectId"].as_str()?))
        })
        .collect();
    items.sort_by_key(|(index, _)| *index);
    items.into_iter().map(|(_, object)| object).collect()
}

async fn backend_node_ids(session: &Session, array: &str) -> crate::Result<Vec<i64>> {
    let params = json!({"objectId": array, "ownProperties": true});
    let properties = session.send("Runtime.getProperties", params).await?;
    let pending = array_items(&properties)
        .into_iter()
        .map(|object| session.enqueue("DOM.describeNode", json!({"objectId": object}), None))
        .collect::<crate::Result<Vec<_>>>()?;
    let mut ids = Vec::with_capacity(pending.len());
    for reply in pending {
        let node = &reply.await?.result["node"];
        let id = node["backendNodeId"].as_i64().ok_or_else(|| {
            BrowserError::protocol("DOM.describeNode", -32000, "the node has no backendNodeId")
        })?;
        ids.push(id);
    }
    Ok(ids)
}

impl Frame {
    pub async fn execute_script(
        &self,
        body: &str,
        args: Vec<ScriptArg>,
        options: ScriptOptions,
    ) -> crate::Result<ScriptValue> {
        let spec = script_spec(&args, options);
        let args = &args;
        let outcome = self
            .page
            .run_op(&self.id, spec, |attempt| {
                let frame = self.at(&attempt.frame);
                async move { frame.execute_script_in(&attempt, body, args, options).await }
            })
            .await?;
        match spec.kind {
            OpKind::Script => Ok(outcome.or_else(|_| ScriptValue::null())),
            _ => outcome.read(),
        }
    }

    pub async fn find_css(
        &self,
        css: &str,
        scope: Option<&Element>,
    ) -> crate::Result<Vec<Element>> {
        self.find(FIND_CSS, css, scope).await
    }

    pub async fn find_xpath(
        &self,
        xpath: &str,
        scope: Option<&Element>,
    ) -> crate::Result<Vec<Element>> {
        self.find(FIND_XPATH, xpath, scope).await
    }

    pub async fn source(&self) -> crate::Result<String> {
        let source = self
            .page
            .run_op(&self.id, OpSpec::FRAME_READ, |attempt| {
                let frame = self.at(&attempt.frame);
                async move { frame.evaluate_in(World::Main, PAGE_SOURCE).await }
            })
            .await?
            .read()?;
        match source {
            Value::String(source) => Ok(source),
            other => Err(malformed(&format!("of the page source probe is {other}"))),
        }
    }

    pub(crate) async fn execute_script_in(
        &self,
        attempt: &OpAttempt,
        body: &str,
        args: &[ScriptArg],
        options: ScriptOptions,
    ) -> crate::Result<ScriptValue> {
        let scope = self.world_scope(options.world).await?;
        let (arguments, nodes) = self.script_arguments(attempt, args, options.world).await?;
        let mut call_args = vec![
            json!({"value": body}),
            json!({"value": arguments}),
            json!({"value": true}),
        ];
        call_args.extend(nodes);
        let result = call_wrapper(&scope, attempt, call_args).await?;
        let (value, refs) = parse_wrapper_result(&result)?;
        self.link_result(value, &refs, &scope.stamp)
    }

    pub(crate) async fn find_css_in(
        &self,
        attempt: &OpAttempt,
        css: &str,
        scope: Option<&Element>,
    ) -> crate::Result<Vec<Element>> {
        self.find_in(attempt, FIND_CSS, css, scope).await
    }

    pub(crate) async fn find_xpath_in(
        &self,
        attempt: &OpAttempt,
        xpath: &str,
        scope: Option<&Element>,
    ) -> crate::Result<Vec<Element>> {
        self.find_in(attempt, FIND_XPATH, xpath, scope).await
    }

    pub(crate) async fn evaluate_in(
        &self,
        world: World,
        declaration: &str,
    ) -> crate::Result<Value> {
        let scope = self.world_scope(world).await?;
        let params = json!({
            "functionDeclaration": declaration,
            "uniqueContextId": &*scope.context.unique_id,
            "returnByValue": true,
        });
        let reply = scope.session.send(CALL_METHOD, params).await?;
        evaluation_result(reply, true)
    }

    fn at(&self, id: &FrameId) -> Frame {
        Frame {
            page: self.page.clone(),
            id: id.clone(),
        }
    }

    async fn world_scope(&self, world: World) -> crate::Result<WorldScope> {
        let stamp = self.stamp()?;
        let session = self
            .page
            .inner
            .connection
            .session(Some(stamp.session.clone()));
        let context = world_context(self, &session, world, &stamp).await?;
        Ok(WorldScope {
            stamp,
            session,
            context,
        })
    }

    fn check_owns(&self, element: &Element) -> crate::Result<()> {
        let same_page = element.frame.page.target_id() == self.page.target_id();
        if same_page && element.frame.id == self.id {
            return Ok(());
        }
        Err(BrowserError::InvalidArgument {
            message: "Element belongs to another frame".to_owned(),
        })
    }

    async fn script_arguments(
        &self,
        attempt: &OpAttempt,
        args: &[ScriptArg],
        world: World,
    ) -> crate::Result<(Value, Vec<Value>)> {
        let mut values = Vec::with_capacity(args.len());
        let mut nodes = Vec::new();
        for arg in args {
            match arg {
                ScriptArg::Json(value) => values.push(value.clone()),
                ScriptArg::Element(element) => {
                    self.check_owns(element)?;
                    let node = element
                        .resolve_in(attempt, world)
                        .await
                        .map_err(|error| element.map_stale(error))?;
                    values.push(json!({ ELEMENT_KEY: nodes.len() }));
                    nodes.push(json!({"objectId": node.object_id}));
                }
            }
        }
        Ok((Value::Array(values), nodes))
    }

    fn link_result(
        &self,
        mut value: Value,
        refs: &[SerializedRef],
        before: &FrameStamp,
    ) -> crate::Result<ScriptValue> {
        let elements = self.mint_nodes(refs, before)?;
        let mut minted = elements.iter();
        let links: Vec<Link> = refs
            .iter()
            .map(|serialized| match serialized {
                SerializedRef::Node { .. } => minted.next().map_or(Link::Unsupported, |element| {
                    Link::Node(element.web_element_id())
                }),
                SerializedRef::Window { context } => Link::Window {
                    key: self.window_key(context),
                    context: context.clone(),
                },
                SerializedRef::Unsupported => Link::Unsupported,
            })
            .collect();
        link_value(&mut value, &links)?;
        Ok(ScriptValue {
            json: value,
            elements,
        })
    }

    fn mint_nodes(
        &self,
        refs: &[SerializedRef],
        before: &FrameStamp,
    ) -> crate::Result<Vec<Element>> {
        let nodes: Vec<(i64, Option<&LoaderId>)> = refs
            .iter()
            .filter_map(|serialized| match serialized {
                SerializedRef::Node {
                    backend_node_id,
                    loader,
                } => Some((*backend_node_id, loader.as_ref())),
                _ => None,
            })
            .collect();
        if nodes.is_empty() {
            return Ok(Vec::new());
        }
        let ids = nodes.iter().map(|(id, _)| *id).collect();
        let minted = mint_elements(self, before, ids, CALL_METHOD)?;
        Ok(minted
            .into_iter()
            .zip(nodes)
            .map(|(element, (_, loader))| match loader {
                Some(loader) => self.adopt(element, loader),
                None => element,
            })
            .collect())
    }

    /// A node from another same-process document (a same-origin child frame) carries that
    /// document's loader; it belongs to the frame that committed it.
    fn adopt(&self, element: Element, loader: &LoaderId) -> Element {
        if element.loader_id == *loader {
            return element;
        }
        let owner = self
            .frame_with_loader(loader)
            .and_then(|frame| frame.stamp().ok().map(|stamp| (frame, stamp)));
        match owner {
            Some((frame, stamp)) if stamp.loader == *loader => {
                Element::new(frame, &stamp, element.backend_node_id, None)
            }
            _ => Element {
                loader_id: loader.clone(),
                ..element
            },
        }
    }

    fn frame_with_loader(&self, loader: &LoaderId) -> Option<Frame> {
        let id = {
            let state = self.page.inner.lock_state();
            let frames = &state.frames;
            let main = frames.main_id().clone();
            let mut candidates =
                std::iter::once(main.clone()).chain(frames.descendants_preorder(&main));
            candidates.find(|id| frames.committed_loader(id).as_ref() == Some(loader))
        }?;
        Some(self.at(&id))
    }

    fn window_key(&self, context: &str) -> &'static str {
        let top = context == self.page.target_id().as_str();
        if !top && self.page.frame(&FrameId::from(context)).is_some() {
            FRAME_KEY
        } else {
            WINDOW_KEY
        }
    }

    async fn find(
        &self,
        declaration: &'static str,
        selector: &str,
        scope: Option<&Element>,
    ) -> crate::Result<Vec<Element>> {
        let spec = OpSpec {
            frame_scoped: scope.is_none(),
            ..OpSpec::FRAME_READ
        };
        self.page
            .run_op(&self.id, spec, |attempt| {
                let frame = self.at(&attempt.frame);
                async move { frame.find_in(&attempt, declaration, selector, scope).await }
            })
            .await?
            .read()
    }

    async fn find_in(
        &self,
        attempt: &OpAttempt,
        declaration: &str,
        selector: &str,
        scope: Option<&Element>,
    ) -> crate::Result<Vec<Element>> {
        let before = self.stamp()?;
        let (session, root) = self.search_root(attempt, scope).await?;
        let group = ObjectGroup::new("call");
        let params = root.call_params(declaration, selector, group.on(&session));
        let reply = session.send(CALL_METHOD, params).await?;
        if let Some(details) = reply.get("exceptionDetails") {
            return Err(selector_error(details));
        }
        let array = reply["result"]["objectId"]
            .as_str()
            .ok_or_else(|| malformed("of the element search is not an array"))?;
        let ids = backend_node_ids(&session, array).await?;
        mint_elements(self, &before, ids, "DOM.describeNode")
    }

    async fn search_root(
        &self,
        attempt: &OpAttempt,
        scope: Option<&Element>,
    ) -> crate::Result<(Session, SearchRoot)> {
        let Some(scope) = scope else {
            let util = self.world_scope(World::Util).await?;
            let context = util.context.unique_id.to_string();
            return Ok((util.session, SearchRoot::Document(context)));
        };
        self.check_owns(scope)?;
        let node = scope
            .resolve_in(attempt, World::Util)
            .await
            .map_err(|error| scope.map_stale(error))?;
        Ok((node.session, SearchRoot::Scope(node.object_id)))
    }
}

async fn call_wrapper(
    scope: &WorldScope,
    attempt: &OpAttempt,
    arguments: Vec<Value>,
) -> crate::Result<Value> {
    let group = ObjectGroup::new("call");
    let params = json!({
        "functionDeclaration": CALL_WRAPPER,
        "uniqueContextId": &*scope.context.unique_id,
        "arguments": arguments,
        "awaitPromise": true,
        "objectGroup": group.on(&scope.session),
        "serializationOptions": {"serialization": "deep"},
    });
    let now = tokio::time::Instant::now();
    let budget = SCRIPT_TIMEOUT.min(attempt.deadline.saturating_duration_since(now));
    let call = scope.session.send(CALL_METHOD, params);
    let reply =
        with_op_deadline(now + SCRIPT_TIMEOUT, call)
            .await
            .map_err(|error| match error {
                BrowserError::Timeout { .. } => BrowserError::ScriptTimeout {
                    seconds: budget.as_secs_f64().ceil() as u64,
                },
                other => other,
            })?;
    evaluation_result(reply, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serialized(response: &str, nodes: Vec<Value>) -> Value {
        let mut list = vec![json!({"type": "string", "value": response})];
        list.extend(nodes);
        json!({"type": "object", "deepSerializedValue": {"type": "array", "value": list}})
    }

    fn header(source: &str) -> &str {
        source.split_inclusive('\n').next().unwrap_or_default()
    }

    fn wrapped_scripts(wrapper: &str) -> &str {
        let start = wrapper
            .find("(function() { ")
            .expect("the wrapper applies call_function.js");
        let end = wrapper
            .rfind("\n, [")
            .expect("the wrapper passes the script arguments");
        &wrapper[start..end]
    }

    #[tokio::test]
    async fn an_object_group_is_released_once_on_every_session_that_used_it() {
        let harness = crate::testing::PageHarness::new().await;
        let session = |id: &str| {
            harness
                .connection
                .session(Some(crate::types::SessionId::from(id)))
        };
        drop(ObjectGroup::new("op"));
        let group = ObjectGroup::new("op");
        let name = group.on(&session("S1")).to_owned();
        assert_eq!(group.on(&session("S2")), name);
        assert_eq!(group.on(&session("S1")), name);
        drop(group);
        harness
            .sent("the release on S2", |command| {
                command.method == "Runtime.releaseObjectGroup"
                    && command.session.as_deref() == Some("S2")
            })
            .await;
        let released: Vec<_> = harness
            .control
            .commands_seen()
            .into_iter()
            .filter(|command| command.method == "Runtime.releaseObjectGroup")
            .map(|command| (command.session, command.params["objectGroup"].clone()))
            .collect();
        assert_eq!(
            released,
            [
                (Some("S1".to_owned()), json!(name)),
                (Some("S2".to_owned()), json!(name))
            ],
            "an unused group sends nothing"
        );
    }

    #[test]
    fn the_call_wrapper_composes_the_scripts_exactly_like_chromedriver() {
        let recorded = include_str!("js/execute_wrapper.js")
            .split_once('\n')
            .map(|(_, wrapper)| wrapper)
            .unwrap();
        let ours = CALL_WRAPPER
            .replace(header(include_str!("js/call_function.js")), "")
            .replace(header(include_str!("js/execute_script.js")), "");
        assert_eq!(wrapped_scripts(&ours), wrapped_scripts(recorded));
        assert!(ours.starts_with("function(script,args,w3c){"));
        assert!(ours.ends_with("\n, [script, args], w3c, nodes]); }"));
    }

    #[test]
    fn wrapper_statuses_map_to_webdriver_errors() {
        let ok = serialized(r#"{"status":0,"value":[1,null]}"#, Vec::new());
        assert_eq!(parse_wrapper_result(&ok).unwrap().0, json!([1, null]));
        let missing_value = serialized(r#"{"status":0}"#, Vec::new());
        assert_eq!(parse_wrapper_result(&missing_value).unwrap().0, Value::Null);
        let stale = serialized(r#"{"status":10,"value":"stale"}"#, Vec::new());
        assert!(matches!(
            parse_wrapper_result(&stale),
            Err(BrowserError::StaleElement)
        ));
        let thrown = serialized(r#"{"status":17,"value":"boom"}"#, Vec::new());
        assert!(matches!(
            parse_wrapper_result(&thrown),
            Err(BrowserError::Javascript { message }) if message == "boom"
        ));
        let shadow = serialized(r#"{"status":66,"value":"detached"}"#, Vec::new());
        assert!(matches!(
            parse_wrapper_result(&shadow),
            Err(BrowserError::Javascript { message }) if message == "detached"
        ));
        let no_status = serialized(r#"{"value":1}"#, Vec::new());
        assert!(matches!(
            parse_wrapper_result(&no_status),
            Err(BrowserError::Protocol { .. })
        ));
        assert!(parse_wrapper_result(&json!({"type": "object"})).is_err());
    }

    #[test]
    fn a_lone_surrogate_in_the_result_is_repaired_instead_of_failing() {
        let result = serialized(r#"{"status":0,"value":"a\ud83d"}"#, Vec::new());
        assert_eq!(parse_wrapper_result(&result).unwrap().0, json!("a\u{FFFD}"));
        let broken = serialized(r#"{"status":0,"value":"#, Vec::new());
        assert!(matches!(
            parse_wrapper_result(&broken),
            Err(BrowserError::Protocol { message, .. }) if message.contains("not valid JSON")
        ));
    }

    #[test]
    fn repeated_nodes_resolve_their_weak_references() {
        let first = json!({"type": "node", "value": {"backendNodeId": 8, "loaderId": "L1"}, "weakLocalObjectReference": 1});
        let again = json!({"type": "node", "weakLocalObjectReference": 1});
        let window = json!({"type": "window", "value": {"context": "T1"}});
        let result = serialized(r#"{"status":0,"value":null}"#, vec![first, again, window]);
        let (_, refs) = parse_wrapper_result(&result).unwrap();
        let node = SerializedRef::Node {
            backend_node_id: 8,
            loader: Some(LoaderId::from("L1")),
        };
        assert_eq!(
            refs,
            vec![
                node.clone(),
                node,
                SerializedRef::Window {
                    context: "T1".to_owned()
                }
            ]
        );
        let orphan = json!({"type": "node", "weakLocalObjectReference": 4});
        assert_eq!(SerializedRef::parse(&orphan), SerializedRef::Unsupported);
    }

    #[test]
    fn placeholders_become_element_and_window_references() {
        let links = vec![
            Link::Node("f.T1.d.L1.e.8".to_owned()),
            Link::Window {
                key: WINDOW_KEY,
                context: "T1".to_owned(),
            },
            Link::Window {
                key: FRAME_KEY,
                context: "F2".to_owned(),
            },
        ];
        let mut value = json!({
            "list": [{ELEMENT_KEY: 0}, {SHADOW_KEY: 0}],
            "top": {WINDOW_KEY: 1},
            "child": {WINDOW_KEY: 2},
            "plain": {"a": [1, 2]},
        });
        link_value(&mut value, &links).unwrap();
        assert_eq!(
            value,
            json!({
                "list": [{ELEMENT_KEY: "f.T1.d.L1.e.8"}, {SHADOW_KEY: "f.T1.d.L1.e.8"}],
                "top": {WINDOW_KEY: "T1"},
                "child": {FRAME_KEY: "F2"},
                "plain": {"a": [1, 2]},
            })
        );
    }

    #[test]
    fn broken_placeholders_are_reported() {
        let links = vec![Link::Node("f.T1.d.L1.e.8".to_owned()), Link::Unsupported];
        for mut value in [
            json!({ELEMENT_KEY: 5}),
            json!({ELEMENT_KEY: "f.T1.d.L1.e.8"}),
            json!({WINDOW_KEY: 0}),
            json!([{ELEMENT_KEY: 1}]),
        ] {
            let error = link_value(&mut value, &links).unwrap_err();
            assert!(
                matches!(error, BrowserError::Protocol { ref method, .. } if method == CALL_METHOD),
                "{error}"
            );
        }
    }

    #[test]
    fn element_arguments_pin_user_scripts_to_their_frame() {
        let json_only = [ScriptArg::Json(json!(1))];
        let user = script_spec(&json_only, ScriptOptions::USER);
        assert_eq!(user.kind, OpKind::Script);
        assert!(user.frame_scoped);
        let probe = script_spec(&json_only, ScriptOptions::PROBE);
        assert_eq!(probe.kind, OpKind::Read);
        assert!(probe.frame_scoped);
        assert_eq!(script_spec(&[], ScriptOptions::INTERNAL).kind, OpKind::Read);
    }

    #[test]
    fn thrown_selector_messages_become_invalid_arguments() {
        let invalid = json!({"text": "Uncaught", "exception": {"type": "string", "value": "invalid selector: bad"}});
        assert!(matches!(
            selector_error(&invalid),
            BrowserError::InvalidArgument { message } if message == "invalid selector: bad"
        ));
        let other = json!({"text": "Uncaught", "exception": {"type": "object", "description": "TypeError: x"}});
        assert!(matches!(
            selector_error(&other),
            BrowserError::Javascript { message } if message == "TypeError: x"
        ));
    }
}
