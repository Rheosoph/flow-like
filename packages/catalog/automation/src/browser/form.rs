#[cfg(feature = "execute")]
use crate::types::handles::AutomationSession;
use crate::types::selectors::Selector;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FormFieldKind {
    #[default]
    Text,
    Select,
    Checkbox,
    Radio,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct FormField {
    /// Field element; a plain string is read as a snapshot ref (`e12`) or a CSS selector.
    #[serde(deserialize_with = "selector_or_string")]
    pub target: Selector,
    /// Text to type, option value or label for selects, `true`/`false` for checkboxes and
    /// radios (empty means checked).
    #[serde(default, deserialize_with = "lenient_string")]
    pub value: String,
    #[serde(default)]
    pub kind: FormFieldKind,
}

fn selector_or_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Selector, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Target {
        Text(String),
        Typed(Selector),
    }
    Ok(match Target::deserialize(deserializer)? {
        Target::Text(text) => Selector::from_css_or_ref(&text),
        Target::Typed(selector) => selector,
    })
}

fn lenient_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match flow_like_types::Value::deserialize(deserializer)? {
        flow_like_types::Value::String(text) => text,
        flow_like_types::Value::Null => String::new(),
        other => other.to_string(),
    })
}

#[cfg(any(feature = "execute", test))]
fn desired_checked(field: &FormField) -> flow_like_types::Result<bool> {
    let checked = match field.value.trim().to_ascii_lowercase().as_str() {
        "" | "true" | "1" | "yes" | "on" | "checked" => true,
        "false" | "0" | "no" | "off" | "unchecked" => false,
        other => {
            return Err(flow_like_types::anyhow!(
                "value '{other}' is not true or false"
            ));
        }
    };
    if field.kind == FormFieldKind::Radio && !checked {
        return Err(flow_like_types::anyhow!(
            "a radio button can only be checked; check another option instead"
        ));
    }
    Ok(checked)
}

#[cfg(any(feature = "execute", test))]
fn describe(selector: &Selector) -> String {
    format!("{:?} '{}'", selector.kind, selector.value)
}

#[cfg(any(feature = "execute", test))]
fn validate_fields(fields: &[FormField]) -> flow_like_types::Result<()> {
    if fields.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Fill Form needs at least one field"
        ));
    }
    for (index, field) in fields.iter().enumerate() {
        let check = || -> flow_like_types::Result<()> {
            if field.target.value.trim().is_empty() {
                return Err(flow_like_types::anyhow!("target is empty"));
            }
            if matches!(field.kind, FormFieldKind::Checkbox | FormFieldKind::Radio) {
                desired_checked(field)?;
            }
            Ok(())
        };
        check().map_err(|error| {
            flow_like_types::anyhow!(
                "Form field {} ({}) is invalid: {error}",
                index + 1,
                describe(&field.target)
            )
        })?;
    }
    Ok(())
}

#[cfg(feature = "execute")]
#[derive(Deserialize, Debug, Default)]
struct Probe {
    visible: bool,
    enabled: bool,
    checked: bool,
    has_label: bool,
    tag: String,
}

#[cfg(feature = "execute")]
const PROBE: &str = r#"
const el = arguments[0];
el.scrollIntoView({ block: 'center', inline: 'nearest' });
const shown = (node) => {
  const rect = node.getBoundingClientRect();
  return node.isConnected && (typeof node.checkVisibility !== 'function' || node.checkVisibility({ checkVisibilityCSS: true })) && rect.width > 0 && rect.height > 0;
};
const label = el.labels ? Array.from(el.labels).find(shown) : null;
const aria = el.getAttribute('aria-checked');
return {
  visible: shown(el),
  enabled: !el.matches(':disabled') && !el.closest('[aria-disabled="true"]'),
  checked: typeof el.checked === 'boolean' ? el.checked : aria === 'true',
  has_label: !!label,
  tag: el.localName,
};
"#;

#[cfg(feature = "execute")]
async fn probe(
    driver: &thirtyfour::WebDriver,
    element: &thirtyfour::WebElement,
) -> flow_like_types::Result<Probe> {
    Ok(driver
        .execute(PROBE, vec![element.to_json()?])
        .await?
        .convert()?)
}

/// Finds the target and waits until it can be used: enabled, and visible unless it is a
/// checkbox or radio with a visible label or script fallback.
#[cfg(feature = "execute")]
async fn actionable(
    context: &ExecutionContext,
    driver: &thirtyfour::WebDriver,
    target: &Selector,
    needs_visible: bool,
    timeout: std::time::Duration,
) -> flow_like_types::Result<(thirtyfour::WebElement, Probe)> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        context.check_cancelled()?;
        let reason = match super::selector::find(driver, target).await {
            Ok(element) => match probe(driver, &element).await {
                Ok(state) if state.enabled && (state.visible || !needs_visible) => {
                    return Ok((element, state));
                }
                Ok(state) if !state.enabled => "the element is disabled".to_string(),
                Ok(_) => "the element is not visible".to_string(),
                Err(error) => error.to_string(),
            },
            Err(error) if super::refs::is_stale_ref_error(&error) => return Err(error),
            Err(error) => error.to_string(),
        };
        if tokio::time::Instant::now() >= deadline {
            return Err(flow_like_types::anyhow!(
                "not ready within {} ms: {reason}",
                timeout.as_millis()
            ));
        }
        crate::rpa::branch::delay(context, std::time::Duration::from_millis(100)).await?;
    }
}

#[cfg(feature = "execute")]
async fn set_checked(
    driver: &thirtyfour::WebDriver,
    element: &thirtyfour::WebElement,
    state: &Probe,
    desired: bool,
) -> flow_like_types::Result<()> {
    if state.checked == desired {
        return Ok(());
    }
    if state.visible {
        element.click().await?;
    } else if state.has_label {
        driver
            .execute(
                "return Array.from(arguments[0].labels).find((label) => { const rect = label.getBoundingClientRect(); return rect.width > 0 && rect.height > 0; });",
                vec![element.to_json()?],
            )
            .await?
            .element()?
            .click()
            .await?;
    } else {
        driver
            .execute("arguments[0].click();", vec![element.to_json()?])
            .await?;
    }
    if probe(driver, element).await?.checked != desired {
        return Err(flow_like_types::anyhow!(
            "clicking did not {} the element",
            if desired { "check" } else { "uncheck" }
        ));
    }
    Ok(())
}

#[cfg(feature = "execute")]
async fn fill_field(
    context: &ExecutionContext,
    driver: &thirtyfour::WebDriver,
    field: &FormField,
    timeout: std::time::Duration,
) -> flow_like_types::Result<()> {
    let toggles = matches!(field.kind, FormFieldKind::Checkbox | FormFieldKind::Radio);
    let (element, state) = actionable(context, driver, &field.target, !toggles, timeout).await?;
    match field.kind {
        FormFieldKind::Text => {
            element.clear().await?;
            if !field.value.is_empty() {
                element.send_keys(field.value.as_str()).await?;
            }
        }
        FormFieldKind::Select => {
            if state.tag != "select" {
                return Err(flow_like_types::anyhow!(
                    "select fields need a <select> element (found <{}>); click custom dropdown options instead",
                    state.tag
                ));
            }
            let select = thirtyfour::components::SelectElement::new(&element).await?;
            if select.select_by_value(&field.value).await.is_err() {
                select
                    .select_by_exact_text(&field.value)
                    .await
                    .map_err(|_| {
                        flow_like_types::anyhow!(
                            "no option has the value or label '{}'",
                            field.value
                        )
                    })?;
            }
        }
        FormFieldKind::Checkbox | FormFieldKind::Radio => {
            set_checked(driver, &element, &state, desired_checked(field)?).await?;
        }
    }
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserFillFormNode {}

impl BrowserFillFormNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserFillFormNode {
    fn get_node(&self) -> Node {
        let mut node = super::manage::base_node(
            "browser_fill_form",
            "Fill Form",
            "Fills several form fields in order: text inputs are cleared and typed, selects pick an option by value or label, checkboxes and radios are set. Each field is scrolled into view and must become visible and enabled within the timeout. Stops at the first failing field.",
        );
        node.category = "Automation/Browser/Input".to_string();
        node.set_flowscript_name("browser", "fillForm");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(7)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "fields",
            "Fields",
            "Fields to fill: target (selector or ref such as e12), value and kind (text, select, checkbox, radio)",
            VariableType::Struct,
        )
        .set_schema::<FormField>()
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));
        node.add_input_pin(
            "timeout_ms",
            "Field Timeout (ms)",
            "Maximum wait per field for it to become visible and enabled",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5000)));
        node.add_output_pin(
            "filled_count",
            "Filled",
            "Number of fields filled",
            VariableType::Integer,
        );
        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let fields: Vec<FormField> = context.evaluate_pin("fields").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;
        if !(0..=300_000).contains(&timeout_ms) {
            return Err(flow_like_types::anyhow!(
                "Field timeout must be 0 to 300000 ms (got {timeout_ms})"
            ));
        }
        validate_fields(&fields)?;
        let timeout = std::time::Duration::from_millis(timeout_ms as u64);
        context.set_pin_value("filled_count", json!(0)).await?;
        let driver = session.get_browser_driver_and_switch(context).await?;
        for (index, field) in fields.iter().enumerate() {
            fill_field(context, &driver, field, timeout)
                .await
                .map_err(|error| {
                    flow_like_types::anyhow!(
                        "Form field {} ({}) failed: {error}",
                        index + 1,
                        describe(&field.target)
                    )
                })?;
            context
                .set_pin_value("filled_count", json!(index + 1))
                .await?;
        }
        drop(driver);
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
pub struct BrowserTypeSecretNode {}

impl BrowserTypeSecretNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserTypeSecretNode {
    fn get_node(&self) -> Node {
        let mut node = super::manage::base_node(
            "browser_type_secret",
            "Type Secret",
            "Types a password or other secret into a field without logging it. The field must become visible and enabled within the timeout.",
        );
        node.category = "Automation/Browser/Input".to_string();
        node.set_flowscript_name("browser", "typeSecret");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "selector",
            "CSS Selector",
            "CSS selector of the field, or a snapshot ref such as e12",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        super::selector::add_locator_pin(&mut node);
        node.add_input_pin(
            "secret",
            "Secret",
            "Value to type; never written to logs",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build())
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "clear",
            "Clear First",
            "Clear the field before typing",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "submit",
            "Submit",
            "Press Enter after typing",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum wait for the field to become visible and enabled",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5000)));
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let secret: String = context.evaluate_pin("secret").await?;
        let clear: bool = context.evaluate_pin("clear").await?;
        let submit: bool = context.evaluate_pin("submit").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;
        if !(0..=300_000).contains(&timeout_ms) {
            return Err(flow_like_types::anyhow!(
                "Timeout must be 0 to 300000 ms (got {timeout_ms})"
            ));
        }
        let driver = session.get_browser_driver_and_switch(context).await?;
        let (element, _) = actionable(
            context,
            &driver,
            &locator,
            true,
            std::time::Duration::from_millis(timeout_ms as u64),
        )
        .await
        .map_err(|error| {
            flow_like_types::anyhow!("Type Secret target {} failed: {error}", describe(&locator))
        })?;
        if clear {
            element.clear().await?;
        }
        element.send_keys(secret.as_str()).await.map_err(|_| {
            flow_like_types::anyhow!(
                "Type Secret could not type into {}; the field rejected keyboard input",
                describe(&locator)
            )
        })?;
        if submit {
            element.send_keys(thirtyfour::Key::Enter).await?;
        }
        drop(driver);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::selectors::SelectorKind;

    fn field(value: flow_like_types::Value) -> FormField {
        flow_like_types::json::from_value(value).unwrap()
    }

    #[test]
    fn fields_accept_refs_strings_and_typed_selectors() {
        let by_ref = field(json!({ "target": "e12", "value": "Ada" }));
        assert_eq!(by_ref.target.kind, SelectorKind::Ref);
        assert_eq!(by_ref.kind, FormFieldKind::Text);
        let by_css = field(json!({ "target": "#email", "value": 42 }));
        assert_eq!(by_css.target.kind, SelectorKind::Css);
        assert_eq!(by_css.value, "42");
        let typed = field(json!({
            "target": { "kind": "Role", "value": "checkbox|Remember me", "confidence": null, "scope": null },
            "value": true,
            "kind": "checkbox"
        }));
        assert_eq!(typed.target.kind, SelectorKind::Role);
        assert_eq!(typed.value, "true");
        assert!(desired_checked(&typed).unwrap());
    }

    #[test]
    fn validation_names_the_failing_field() {
        let fields = vec![
            field(json!({ "target": "e1", "value": "x" })),
            field(json!({ "target": "e2", "value": "maybe", "kind": "checkbox" })),
        ];
        let error = validate_fields(&fields).unwrap_err().to_string();
        assert!(
            error.starts_with("Form field 2 (Ref 'e2') is invalid"),
            "{error}"
        );
        assert!(validate_fields(&[]).is_err());
        let radio = field(json!({ "target": "e3", "value": "false", "kind": "radio" }));
        assert!(validate_fields(&[radio]).is_err());
        let blank = field(json!({ "target": " ", "value": "x" }));
        assert!(validate_fields(&[blank]).is_err());
        let unchecked = field(json!({ "target": "e4", "value": "off", "kind": "checkbox" }));
        assert!(!desired_checked(&unchecked).unwrap());
    }
}
