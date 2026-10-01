#[cfg(feature = "execute")]
use super::driver::PageContext;
#[cfg(feature = "execute")]
use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_browser::{Element, script::ScriptArg};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Element box in CSS pixels relative to the viewport.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct ElementBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct ElementState {
    pub visible: bool,
    pub enabled: bool,
    /// Checked checkbox/radio, selected option, or aria-checked/pressed/selected="true".
    pub checked: bool,
    /// Enabled, not read-only text input, textarea, select or contenteditable.
    pub editable: bool,
    pub focused: bool,
    pub in_viewport: bool,
    pub bounds: ElementBounds,
    pub tag: String,
    /// Visible text, or the value of form fields (empty for password fields).
    pub text: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct ListedElement {
    #[serde(default)]
    pub index: usize,
    /// Snapshot ref, when a browser snapshot exists for this page.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub element_ref: Option<String>,
    pub tag: String,
    pub text: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    pub bounds: ElementBounds,
    pub visible: bool,
}

#[cfg(feature = "execute")]
const VISIBLE_JS: &str = "const visible = (el, rect) => el.isConnected && (typeof el.checkVisibility !== 'function' || el.checkVisibility({ checkVisibilityCSS: true })) && getComputedStyle(el).visibility !== 'hidden' && rect.width > 0 && rect.height > 0;";

#[cfg(feature = "execute")]
fn state_script() -> String {
    format!(
        r#"{VISIBLE_JS}
const el = arguments[0];
const rect = el.getBoundingClientRect();
const tag = el.localName;
const type = (el.getAttribute('type') || '').toLowerCase();
const disabled = el.matches(':disabled') || !!el.closest('[aria-disabled="true"]');
const checkable = tag === 'input' && (type === 'checkbox' || type === 'radio');
const aria = el.getAttribute('aria-checked') ?? el.getAttribute('aria-pressed') ?? el.getAttribute('aria-selected');
const checked = checkable ? el.checked : tag === 'option' ? el.selected : aria === 'true';
const textInput = (tag === 'input' && !['checkbox', 'radio', 'button', 'submit', 'reset', 'image', 'file', 'hidden', 'range', 'color'].includes(type)) || tag === 'textarea';
const editable = !disabled && ((textInput && !el.readOnly) || tag === 'select' || el.isContentEditable);
const root = el.getRootNode();
const focused = el === (root.activeElement || document.activeElement);
const isVisible = visible(el, rect);
const inViewport = isVisible && rect.bottom > 0 && rect.right > 0 && rect.top < window.innerHeight && rect.left < window.innerWidth;
const raw = type === 'password' ? '' : (textInput || tag === 'select') ? String(el.value ?? '') : (el.innerText ?? el.textContent ?? '');
return {{
  visible: isVisible, enabled: !disabled, checked: !!checked, editable, focused, in_viewport: inViewport,
  bounds: {{ x: rect.x, y: rect.y, width: rect.width, height: rect.height }},
  tag, text: raw.trim().slice(0, 4000),
}};"#
    )
}

#[cfg(feature = "execute")]
fn list_script() -> String {
    format!(
        r#"{VISIBLE_JS}
const names = arguments[0];
return Array.from(arguments).slice(1).map((el) => {{
  const rect = el.getBoundingClientRect();
  const attributes = {{}};
  for (const name of names) {{
    const value = el.getAttribute(name);
    if (value !== null) attributes[name] = value;
  }}
  return {{
    tag: el.localName,
    text: (el.innerText ?? el.textContent ?? '').replace(/\s+/g, ' ').trim().slice(0, 500),
    attributes,
    bounds: {{ x: rect.x, y: rect.y, width: rect.width, height: rect.height }},
    visible: visible(el, rect),
  }};
}});"#
    )
}

#[cfg(feature = "execute")]
pub(crate) async fn element_state(
    page: &PageContext,
    element: &Element,
) -> flow_like_types::Result<ElementState> {
    let state = page
        .probe(&state_script(), vec![ScriptArg::Element(element.clone())])
        .await
        .map_err(|error| flow_like_types::anyhow!("Failed to read element state: {error}"))?
        .into_json();
    flow_like_types::json::from_value(state).map_err(|error| {
        flow_like_types::anyhow!("Element state probe returned an unexpected value: {error}")
    })
}

#[cfg(feature = "execute")]
async fn describe_elements(
    page: &PageContext,
    attributes: &[String],
    elements: &[Element],
) -> flow_like_types::Result<Vec<ListedElement>> {
    if elements.is_empty() {
        return Ok(Vec::new());
    }
    let mut arguments = vec![ScriptArg::Json(json!(attributes))];
    arguments.extend(elements.iter().cloned().map(ScriptArg::Element));
    let described = page
        .probe(&list_script(), arguments)
        .await
        .map_err(|error| flow_like_types::anyhow!("Failed to describe elements: {error}"))?
        .into_json();
    flow_like_types::json::from_value(described).map_err(|error| {
        flow_like_types::anyhow!("Element listing probe returned an unexpected value: {error}")
    })
}

fn add_target_pins(node: &mut Node) {
    node.add_input_pin(
        "selector",
        "CSS Selector",
        "CSS selector, or a snapshot ref such as e12",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    super::selector::add_locator_pin(node);
}

fn state_node(id: &str, title: &str, description: &str, flowscript: &str) -> Node {
    let mut node = super::manage::base_node(id, title, description);
    node.category = "Automation/Browser/Extract".to_string();
    node.set_flowscript_name("browser", flowscript);
    node.set_scores(
        flow_like::flow::node::NodeScores::new()
            .set_privacy(5)
            .set_security(7)
            .set_performance(8)
            .set_governance(7)
            .set_reliability(8)
            .set_cost(10)
            .build(),
    );
    add_target_pins(&mut node);
    node
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserGetElementStateNode {}

impl BrowserGetElementStateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetElementStateNode {
    fn get_node(&self) -> Node {
        let mut node = state_node(
            "browser_get_element_state",
            "Get Element State",
            "Reads whether an element is visible, enabled, checked, editable, focused and in the viewport, plus its box, tag and text. Fails when no element matches.",
            "getElementState",
        );
        for (name, title, description) in [
            ("visible", "Visible", "Rendered with a non-empty box"),
            ("enabled", "Enabled", "Not disabled"),
            ("checked", "Checked", "Checked, selected or pressed"),
            ("editable", "Editable", "Accepts typed input"),
            ("focused", "Focused", "Has keyboard focus"),
            (
                "in_viewport",
                "In Viewport",
                "Visible and intersecting the viewport",
            ),
        ] {
            node.add_output_pin(name, title, description, VariableType::Boolean);
        }
        node.add_output_pin(
            "bounds",
            "Bounds",
            "Box in CSS pixels relative to the viewport",
            VariableType::Struct,
        )
        .set_schema::<ElementBounds>();
        node.add_output_pin("tag", "Tag", "Lowercase tag name", VariableType::String);
        node.add_output_pin(
            "text",
            "Text",
            "Visible text, or the field value (empty for passwords)",
            VariableType::String,
        );
        node.add_output_pin("state", "State", "All state values", VariableType::Struct)
            .set_schema::<ElementState>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let page = session.browser_page(context).await?;
        let element = super::selector::find_element(&page, &locator)
            .await
            .map_err(|error| {
                flow_like_types::anyhow!(
                    "Get Element State found no element for {:?} '{}': {error}",
                    locator.kind,
                    locator.value
                )
            })?;
        let state = element_state(&page, &element).await?;
        drop(page);
        for (name, value) in [
            ("visible", state.visible),
            ("enabled", state.enabled),
            ("checked", state.checked),
            ("editable", state.editable),
            ("focused", state.focused),
            ("in_viewport", state.in_viewport),
        ] {
            context.set_pin_value(name, json!(value)).await?;
        }
        context.set_pin_value("bounds", json!(state.bounds)).await?;
        context.set_pin_value("tag", json!(state.tag)).await?;
        context.set_pin_value("text", json!(state.text)).await?;
        context.set_pin_value("state", json!(state)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserCountElementsNode {}

impl BrowserCountElementsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserCountElementsNode {
    fn get_node(&self) -> Node {
        let mut node = state_node(
            "browser_count_elements",
            "Count Elements",
            "Counts the elements matching a selector (0 when none match).",
            "countElements",
        );
        node.add_output_pin(
            "count",
            "Count",
            "Number of matching elements",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let page = session.browser_page(context).await?;
        let count = super::selector::find_elements(&page, &locator).await?.len();
        drop(page);
        context.set_pin_value("count", json!(count)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserListElementsNode {}

impl BrowserListElementsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserListElementsNode {
    fn get_node(&self) -> Node {
        let mut node = state_node(
            "browser_list_elements",
            "List Elements",
            "Lists elements matching a selector with their text, chosen attributes, box and visibility. When a browser snapshot exists for the page, each element, including elements inside frames, also gets a ref usable as a Ref selector.",
            "listElements",
        );
        node.add_input_pin(
            "attributes",
            "Attributes",
            "Attribute names to include for each element",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([
            "id",
            "name",
            "type",
            "href",
            "aria-label",
            "placeholder",
            "role"
        ])));
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum elements returned",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50)));
        node.add_output_pin(
            "elements",
            "Elements",
            "Matching elements in document order",
            VariableType::Struct,
        )
        .set_schema::<ListedElement>()
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "count",
            "Count",
            "Number of matches before the limit",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let attributes: Vec<String> = context.evaluate_pin("attributes").await?;
        let limit: i64 = context.evaluate_pin("limit").await?;
        if !(1..=1000).contains(&limit) {
            return Err(flow_like_types::anyhow!(
                "List Elements limit must be 1 to 1000 (got {limit})"
            ));
        }
        let page = session.browser_page(context).await?;
        let found = super::selector::find_elements(&page, &locator).await?;
        let count = found.len();
        let elements: Vec<Element> = found.into_iter().take(limit as usize).collect();
        let mut listed = describe_elements(&page, &attributes, &elements).await?;
        let refs = if locator.kind == crate::types::selectors::SelectorKind::Ref {
            vec![crate::types::selectors::normalize_ref(&locator.value)]
        } else {
            super::driver::refs_for_elements(&page, &elements)
        };
        drop(page);
        for (index, (element, reference)) in listed.iter_mut().zip(refs).enumerate() {
            element.index = index;
            element.element_ref = reference;
        }
        context.set_pin_value("elements", json!(listed)).await?;
        context.set_pin_value("count", json!(count)).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the execute feature"
        ))
    }
}
