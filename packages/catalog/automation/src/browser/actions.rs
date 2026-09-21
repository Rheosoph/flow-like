use super::manage::base_node;
use crate::llm::plan_actions::ActionPlan;
#[cfg(any(feature = "execute", test))]
use crate::llm::plan_actions::PlannedAction;
#[cfg(feature = "execute")]
use crate::types::handles::AutomationSession;
use crate::types::selectors::Selector;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BrowserRightClickNode {}
impl BrowserRightClickNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserRightClickNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_right_click",
            "Right Click Element",
            "Opens the context menu for an element.",
        );
        node.set_flowscript_name("browser", "rightClick");
        super::selector::add_locator_pin(&mut node);
        node.add_input_pin(
            "selector",
            "CSS Selector",
            "Legacy CSS selector",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let driver = session.get_browser_driver_and_switch(context).await?;
        let element = super::selector::find(&driver, &locator).await?;
        click_with_modifiers(&driver, &element, "right", &[]).await?;
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
pub struct BrowserDragNode {}
impl BrowserDragNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserDragNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_drag",
            "Drag Element",
            "Drags an element to another element.",
        );
        node.set_flowscript_name("browser", "drag");
        node.add_input_pin("source", "Source", "Element to drag", VariableType::Struct)
            .set_schema::<Selector>();
        node.add_input_pin("target", "Target", "Drop target", VariableType::Struct)
            .set_schema::<Selector>();
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let source: Selector = context.evaluate_pin("source").await?;
        let target: Selector = context.evaluate_pin("target").await?;
        let driver = session.get_browser_driver_and_switch(context).await?;
        let source = super::selector::find(&driver, &source).await?;
        let target = super::selector::find(&driver, &target).await?;
        let result = driver
            .action_chain()
            .drag_and_drop_element(&source, &target)
            .perform()
            .await;
        let reset = driver.action_chain().reset_actions().await;
        result?;
        reset?;
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
pub struct BrowserKeyChordNode {}
impl BrowserKeyChordNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserKeyChordNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_key_chord",
            "Key Chord",
            "Presses a key while holding browser modifier keys.",
        );
        node.set_flowscript_name("browser", "keyChord");
        node.add_input_pin(
            "key",
            "Key",
            "Character or key name such as Enter",
            VariableType::String,
        );
        node.add_input_pin(
            "modifiers",
            "Modifiers",
            "Control, Shift, Alt, or Meta",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let key: String = context.evaluate_pin("key").await?;
        let modifiers: Vec<String> = context.evaluate_pin("modifiers").await?;
        let driver = session.get_browser_driver_and_switch(context).await?;
        key_chord(&driver, &key, &modifiers).await?;
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
pub struct BrowserExecutePlanNode {}
impl BrowserExecutePlanNode {
    pub fn new() -> Self {
        Self {}
    }
}
#[async_trait]
impl NodeLogic for BrowserExecutePlanNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "browser_execute_plan",
            "Execute Browser Action Plan",
            "Executes a validated LLM browser plan in order, stopping on the first failed action.",
        );
        node.set_flowscript_name("browser", "executePlan");
        node.add_input_pin(
            "plan",
            "Plan",
            "Plan from LLM Plan Actions with CSS or typed selectors",
            VariableType::Struct,
        )
        .set_schema::<ActionPlan>();
        node.add_input_pin(
            "max_actions",
            "Maximum Actions",
            "Maximum actions accepted in one plan",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_input_pin(
            "action_timeout_ms",
            "Action Timeout",
            "Maximum time per action in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30000)));
        node.add_output_pin(
            "executed_count",
            "Executed",
            "Number of completed actions",
            VariableType::Integer,
        );
        node.set_long_running(true);
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        let plan: ActionPlan = context.evaluate_pin("plan").await?;
        let max_actions: i64 = context.evaluate_pin("max_actions").await?;
        let timeout: i64 = context.evaluate_pin("action_timeout_ms").await?;
        if !(1..=1000).contains(&max_actions) || timeout <= 0 {
            return Err(flow_like_types::anyhow!(
                "Action limit must be 1 to 1000 and timeout positive"
            ));
        }
        validate_plan(&plan, max_actions as usize)?;
        context.set_pin_value("executed_count", json!(0)).await?;
        if plan
            .actions
            .first()
            .is_some_and(|action| action.action_type == "navigate")
        {
            session.browser_frame_selectors.clear();
        }
        let driver = session.get_browser_driver_and_switch(context).await?;
        for (index, action) in plan.actions.iter().enumerate() {
            context.check_cancelled()?;
            let cancellation = context.get_cancellation_token();
            let result = tokio::select! {
                biased;
                _ = async { if let Some(token) = cancellation { token.cancelled().await } else { std::future::pending::<()>().await } } => Err(flow_like_types::anyhow!("Execution was cancelled")),
                result = tokio::time::timeout(std::time::Duration::from_millis(timeout as u64), execute_action(context, &driver, action)) => result.map_err(|_| flow_like_types::anyhow!("Plan action {} timed out", index + 1)).and_then(|result| result),
            };
            if result.is_err() {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    driver.action_chain().reset_actions(),
                )
                .await;
            }
            result?;
            if action.action_type == "navigate" {
                session.browser_frame_selectors.clear();
            }
            context
                .set_pin_value("executed_count", json!(index + 1))
                .await?;
        }
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

#[cfg(any(feature = "execute", test))]
fn validate_plan(plan: &ActionPlan, max_actions: usize) -> flow_like_types::Result<()> {
    if !plan.goal_understood {
        return Err(flow_like_types::anyhow!(
            "The planner did not understand the goal"
        ));
    }
    if plan.actions.len() > max_actions {
        return Err(flow_like_types::anyhow!(
            "Plan exceeds maximum action count"
        ));
    }
    for action in &plan.actions {
        match action.action_type.as_str() {
            "click" | "double_click" | "right_click" => {
                action_selector(action)?;
                action_modifiers(action)?;
                action_button(action)?;
            }
            "hover" | "scroll" | "check" | "uncheck" => {
                action_selector(action)?;
            }
            "type" | "fill" => {
                action_selector(action)?;
                text_parameter(action, "text")?;
            }
            "select" => {
                action_selector(action)?;
                text_parameter(action, "value")?;
            }
            "press" => {
                validate_key(text_parameter(action, "key")?)?;
                action_modifiers(action)?;
                if !action.target.is_empty() || action.parameters.get("selector").is_some() {
                    action_selector(action)?;
                }
            }
            "navigate" => {
                let url = flow_like_types::reqwest::Url::parse(text_parameter(action, "url")?)?;
                if !matches!(url.scheme(), "http" | "https") {
                    return Err(flow_like_types::anyhow!(
                        "Plan navigation requires an HTTP or HTTPS URL"
                    ));
                }
            }
            "wait" => {
                wait_duration(action)?;
            }
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unsupported browser action: {}",
                    other
                ));
            }
        }
    }
    Ok(())
}
#[cfg(any(feature = "execute", test))]
fn validate_key(key: &str) -> flow_like_types::Result<()> {
    if key.chars().count() == 1
        || matches!(
            key.to_ascii_lowercase().as_str(),
            "enter"
                | "return"
                | "tab"
                | "escape"
                | "esc"
                | "backspace"
                | "delete"
                | "arrowup"
                | "up"
                | "arrowdown"
                | "down"
                | "arrowleft"
                | "left"
                | "arrowright"
                | "right"
                | "home"
                | "end"
                | "pageup"
                | "pagedown"
                | "space"
        )
    {
        Ok(())
    } else {
        Err(flow_like_types::anyhow!("Unknown browser key"))
    }
}
#[cfg(any(feature = "execute", test))]
fn action_modifiers(action: &PlannedAction) -> flow_like_types::Result<Vec<String>> {
    let modifiers: Vec<String> = action
        .parameters
        .get("modifiers")
        .map(|value| flow_like_types::json::from_value(value.clone()))
        .transpose()?
        .unwrap_or_default();
    for modifier in &modifiers {
        if !matches!(
            modifier.to_ascii_lowercase().as_str(),
            "ctrl" | "control" | "shift" | "alt" | "meta" | "cmd" | "command" | "win"
        ) {
            return Err(flow_like_types::anyhow!(
                "Unknown browser modifier: {modifier}"
            ));
        }
    }
    Ok(modifiers)
}
#[cfg(any(feature = "execute", test))]
fn action_button(action: &PlannedAction) -> flow_like_types::Result<&str> {
    let button = match action.parameters.get("button") {
        Some(value) => value
            .as_str()
            .ok_or_else(|| flow_like_types::anyhow!("Click button must be a string"))?,
        None if action.action_type == "right_click" => "right",
        None => "left",
    };
    if !matches!(button, "left" | "middle" | "right") {
        return Err(flow_like_types::anyhow!("Unknown click button"));
    }
    Ok(button)
}
#[cfg(any(feature = "execute", test))]
fn text_parameter<'a>(action: &'a PlannedAction, name: &str) -> flow_like_types::Result<&'a str> {
    action
        .parameters
        .get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Action {} requires string parameter {}",
                action.action_type,
                name
            )
        })
}
#[cfg(any(feature = "execute", test))]
fn wait_duration(action: &PlannedAction) -> flow_like_types::Result<u64> {
    let duration = action
        .parameters
        .get("duration_ms")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| flow_like_types::anyhow!("Wait requires nonnegative duration_ms"))?;
    if duration > 300000 {
        return Err(flow_like_types::anyhow!(
            "A wait cannot exceed five minutes"
        ));
    }
    Ok(duration)
}
#[cfg(any(feature = "execute", test))]
fn action_selector(action: &PlannedAction) -> flow_like_types::Result<Selector> {
    let selector = match action.parameters.get("selector") {
        Some(value) if value.is_object() => flow_like_types::json::from_value(value.clone())?,
        Some(value) if value.is_string() => Selector::css(value.as_str().unwrap()),
        Some(_) => {
            return Err(flow_like_types::anyhow!(
                "selector must be a string or typed Selector"
            ));
        }
        None => Selector::css(&action.target),
    };
    if selector.value.is_empty() {
        return Err(flow_like_types::anyhow!("Action requires a selector"));
    }
    if selector.kind == crate::types::selectors::SelectorKind::Image {
        return Err(flow_like_types::anyhow!(
            "Image selectors cannot be used for browser actions"
        ));
    }
    Ok(selector)
}
#[cfg(feature = "execute")]
fn browser_key(name: &str) -> flow_like_types::Result<thirtyfour::common::keys::TypingData> {
    validate_key(name)?;
    use thirtyfour::Key;
    let key = match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "escape" | "esc" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "arrowup" | "up" => Key::Up,
        "arrowdown" | "down" => Key::Down,
        "arrowleft" | "left" => Key::Left,
        "arrowright" | "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "space" => Key::Space,
        _ if name.chars().count() == 1 => return Ok(name.into()),
        _ => return Err(flow_like_types::anyhow!("Unknown browser key")),
    };
    Ok(key.into())
}
#[cfg(feature = "execute")]
pub(crate) async fn key_chord(
    driver: &thirtyfour::WebDriver,
    key: &str,
    modifiers: &[String],
) -> flow_like_types::Result<()> {
    use thirtyfour::Key;
    let keys = modifiers
        .iter()
        .map(|modifier| match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Ok(Key::Control),
            "shift" => Ok(Key::Shift),
            "alt" => Ok(Key::Alt),
            "meta" | "cmd" | "command" | "win" => Ok(Key::Meta),
            _ => Err(flow_like_types::anyhow!(
                "Unknown browser modifier: {}",
                modifier
            )),
        })
        .collect::<flow_like_types::Result<Vec<_>>>()?;
    let key = browser_key(key)?;
    let mut chain = driver.action_chain();
    for modifier in &keys {
        chain = chain.key_down(modifier.clone());
    }
    chain = chain.send_keys(key);
    for modifier in keys.iter().rev() {
        chain = chain.key_up(modifier.clone());
    }
    let result = chain.perform().await;
    let reset = driver.action_chain().reset_actions().await;
    result?;
    reset?;
    Ok(())
}
#[cfg(feature = "execute")]
async fn execute_action(
    context: &ExecutionContext,
    driver: &thirtyfour::WebDriver,
    action: &PlannedAction,
) -> flow_like_types::Result<()> {
    match action.action_type.as_str() {
        "wait" => {
            return crate::rpa::branch::delay(
                context,
                std::time::Duration::from_millis(wait_duration(action)?),
            )
            .await;
        }
        "navigate" => {
            driver.goto(text_parameter(action, "url")?).await?;
            return Ok(());
        }
        "press" => {
            let modifiers = action_modifiers(action)?;
            if !action.target.is_empty() || action.parameters.get("selector").is_some() {
                super::selector::find(driver, &action_selector(action)?)
                    .await?
                    .focus()
                    .await?;
            }
            return key_chord(driver, text_parameter(action, "key")?, &modifiers).await;
        }
        _ => {}
    }
    let element = super::selector::find(driver, &action_selector(action)?).await?;
    match action.action_type.as_str() {
        "click" | "double_click" | "right_click" => {
            click_with_modifiers_count(
                driver,
                &element,
                action_button(action)?,
                &action_modifiers(action)?,
                if action.action_type == "double_click" {
                    2
                } else {
                    1
                },
            )
            .await?;
        }
        "type" | "fill" => {
            if action.action_type == "fill" {
                element.clear().await?;
            }
            element.send_keys(text_parameter(action, "text")?).await?;
        }
        "hover" => {
            driver
                .action_chain()
                .move_to_element_center(&element)
                .perform()
                .await?
        }
        "scroll" => element.scroll_into_view().await?,
        "select" => {
            thirtyfour::components::SelectElement::new(&element)
                .await?
                .select_by_value(text_parameter(action, "value")?)
                .await?
        }
        "check" | "uncheck" => {
            if element.is_selected().await? != (action.action_type == "check") {
                element.click().await?;
            }
        }
        _ => return Err(flow_like_types::anyhow!("Unsupported action")),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn plan(actions: Vec<PlannedAction>) -> ActionPlan {
        ActionPlan {
            goal_understood: true,
            current_state_assessment: String::new(),
            actions,
            success_criteria: vec![],
            potential_obstacles: vec![],
            confidence: 1.0,
        }
    }
    fn action(kind: &str, parameters: flow_like_types::Value) -> PlannedAction {
        PlannedAction {
            action_type: kind.into(),
            target: "#submit".into(),
            parameters,
            reasoning: String::new(),
            expected_result: String::new(),
        }
    }
    #[test]
    fn validates_entire_plan_before_any_action() {
        assert!(
            validate_plan(
                &plan(vec![
                    action("click", json!({})),
                    action("custom_js", json!({}))
                ]),
                10
            )
            .is_err()
        );
        assert!(validate_plan(&plan(vec![action("wait", json!({"duration_ms":-1}))]), 10).is_err());
        assert!(validate_plan(&plan(vec![action("type", json!({}))]), 10).is_err());
        assert!(validate_plan(&plan(vec![action("click", json!({}))]), 0).is_err());
        assert!(validate_plan(&plan(vec![action("fill", json!({"text":"hello"}))]), 10).is_ok());
        assert!(validate_plan(&plan(vec![action("press", json!({"key":"bad-key"}))]), 10).is_err());
        assert!(
            validate_plan(
                &plan(vec![action(
                    "press",
                    json!({"key":"Enter","modifiers":["unknown"]})
                )]),
                10
            )
            .is_err()
        );
        assert!(
            validate_plan(
                &plan(vec![action(
                    "navigate",
                    json!({"url":"javascript:alert(1)"})
                )]),
                10
            )
            .is_err()
        );
        assert!(
            validate_plan(
                &plan(vec![action("click", json!({"button":"invalid"}))]),
                10
            )
            .is_err()
        );
    }
}

#[cfg(feature = "execute")]
pub(crate) async fn click_with_modifiers(
    driver: &thirtyfour::WebDriver,
    element: &thirtyfour::WebElement,
    button: &str,
    modifiers: &[String],
) -> flow_like_types::Result<()> {
    click_with_modifiers_count(driver, element, button, modifiers, 1).await
}
#[cfg(feature = "execute")]
pub(crate) async fn click_with_modifiers_count(
    driver: &thirtyfour::WebDriver,
    element: &thirtyfour::WebElement,
    button: &str,
    modifiers: &[String],
    count: usize,
) -> flow_like_types::Result<()> {
    use thirtyfour::Key;
    let button = match button {
        "left" => 0,
        "middle" => 1,
        "right" => 2,
        _ => {
            return Err(flow_like_types::anyhow!(
                "Supported click buttons are left, middle, and right"
            ));
        }
    };
    let keys = modifiers
        .iter()
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Ok(Key::Control),
            "shift" => Ok(Key::Shift),
            "alt" => Ok(Key::Alt),
            "meta" | "cmd" | "command" | "win" => Ok(Key::Meta),
            _ => Err(flow_like_types::anyhow!("Unknown click modifier")),
        })
        .collect::<flow_like_types::Result<Vec<_>>>()?;
    let pause = json!({"type":"pause","duration":0});
    let mut key_actions = Vec::new();
    let mut pointer_actions = Vec::new();
    for key in &keys {
        key_actions.push(json!({"type":"keyDown","value":char::from(key.clone()).to_string()}));
        pointer_actions.push(pause.clone());
    }
    pointer_actions.push(json!({"type":"pointerMove","duration":0,"origin":{"element-6066-11e4-a52e-4f735466cecf":element.element_id()},"x":0,"y":0}));
    key_actions.push(pause.clone());
    for _ in 0..count {
        pointer_actions.push(json!({"type":"pointerDown","button":button}));
        pointer_actions.push(json!({"type":"pointerUp","button":button}));
        key_actions.extend([pause.clone(), pause.clone()]);
    }
    for key in keys.iter().rev() {
        key_actions.push(json!({"type":"keyUp","value":char::from(key.clone()).to_string()}));
        pointer_actions.push(pause.clone());
    }
    let actions = thirtyfour::common::command::Actions::from(json!([
        {"type":"key","id":"automation-keyboard","actions":key_actions},
        {"type":"pointer","id":"automation-mouse","parameters":{"pointerType":"mouse"},"actions":pointer_actions},
    ]));
    let result = driver
        .handle
        .cmd(thirtyfour::common::command::Command::PerformActions(
            actions,
        ))
        .await;
    let reset = driver.action_chain().reset_actions().await;
    result?;
    reset?;
    Ok(())
}
