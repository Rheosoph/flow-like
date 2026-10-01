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
        let page = session.browser_page(context).await?;
        let element = super::selector::find_element(&page, &locator).await?;
        super::driver::click_element(&page, &element, "right", &[], 1).await?;
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
        let page = session.browser_page(context).await?;
        let source = super::selector::find_element(&page, &source).await?;
        let target = super::selector::find_element(&page, &target).await?;
        if let Err(error) = source.drag_to(&target).await {
            release_input(&page).await;
            return Err(error.into());
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
        let page = session.browser_page(context).await?;
        super::driver::key_chord(&page, &key, &modifiers).await?;
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
            "Executes a validated LLM browser plan in order, stopping on the first failed action. Navigate actions follow the session navigation policy (HTTP and HTTPS only when none is set), including the final URL after redirects. A 'select' action fails when no option has the value.",
        );
        node.set_version(2);
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
        let policy = super::policy::session_policy(
            context,
            &session,
            super::policy::NavigationPolicy::plan_default(),
        )
        .await;
        validate_plan(&plan, max_actions as usize, &policy)?;
        context.set_pin_value("executed_count", json!(0)).await?;
        if plan
            .actions
            .first()
            .is_some_and(|action| action.action_type == "navigate")
        {
            session.browser_frame_selectors.clear();
        }
        let timeout = std::time::Duration::from_millis(timeout as u64);
        let operation = session.browser_page(context).await?;
        let mut top_frame: Option<super::driver::PageContext> = None;
        for (index, action) in plan.actions.iter().enumerate() {
            context.check_cancelled()?;
            let page = top_frame.as_ref().unwrap_or(&*operation);
            run_planned_action(context, page, action, &policy, timeout, index).await?;
            if action.action_type == "navigate" && !session.browser_frame_selectors.is_empty() {
                session.browser_frame_selectors.clear();
                top_frame = Some(main_frame_view(&operation));
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
fn validate_plan(
    plan: &ActionPlan,
    max_actions: usize,
    policy: &super::policy::NavigationPolicy,
) -> flow_like_types::Result<()> {
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
                let url = super::policy::NavigationPolicy::parse(text_parameter(action, "url")?)?;
                policy.check_static(&url)?;
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
async fn run_planned_action(
    context: &ExecutionContext,
    page: &super::driver::PageContext,
    action: &PlannedAction,
    policy: &super::policy::NavigationPolicy,
    timeout: std::time::Duration,
    index: usize,
) -> flow_like_types::Result<()> {
    let cancellation = context.get_cancellation_token();
    let result = tokio::select! {
        biased;
        _ = async { if let Some(token) = cancellation { token.cancelled().await } else { std::future::pending::<()>().await } } => Err(flow_like_types::anyhow!("Execution was cancelled")),
        result = tokio::time::timeout(timeout, execute_action(context, page, action, policy)) => result.map_err(|_| flow_like_types::anyhow!("Plan action {} timed out", index + 1)).and_then(|result| result),
    };
    if result.is_err() {
        release_input(page).await;
    }
    result
}
/// Releases held keys and buttons after a failed action, bounded so a hung renderer
/// cannot delay the reported error.
#[cfg(feature = "execute")]
async fn release_input(page: &super::driver::PageContext) {
    let _ =
        tokio::time::timeout(std::time::Duration::from_secs(2), page.page.release_input()).await;
}
/// The same page with the main frame current, as WebDriver Navigate To leaves it. The plan
/// keeps its operation lock, so no parallel branch runs between the navigate and the next action.
#[cfg(feature = "execute")]
fn main_frame_view(page: &super::driver::PageContext) -> super::driver::PageContext {
    super::driver::PageContext::new(
        page.browser.clone(),
        page.page.clone(),
        page.slot.clone(),
        Vec::new(),
    )
}
#[cfg(feature = "execute")]
async fn execute_action(
    context: &ExecutionContext,
    page: &super::driver::PageContext,
    action: &PlannedAction,
    policy: &super::policy::NavigationPolicy,
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
            let url = text_parameter(action, "url")?;
            let target = policy.check(url).await?;
            page.page.goto(target.as_str()).await?;
            super::policy::verify_landing(page, policy, url).await?;
            return Ok(());
        }
        "press" => {
            let modifiers = action_modifiers(action)?;
            if !action.target.is_empty() || action.parameters.get("selector").is_some() {
                super::selector::find_element(page, &action_selector(action)?)
                    .await?
                    .focus()
                    .await?;
            }
            return super::driver::key_chord(page, text_parameter(action, "key")?, &modifiers)
                .await;
        }
        _ => {}
    }
    let element = super::selector::find_element(page, &action_selector(action)?).await?;
    match action.action_type.as_str() {
        "click" | "double_click" | "right_click" => {
            super::driver::click_element(
                page,
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
            element.hover().await?;
        }
        "scroll" => element.scroll_into_view().await?,
        "select" => {
            element
                .select_by_value(text_parameter(action, "value")?)
                .await?
        }
        "check" | "uncheck" => {
            if element.is_selected().await? != (action.action_type == "check") {
                element.element_click().await?;
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
    fn validate(plan: &ActionPlan, max_actions: usize) -> flow_like_types::Result<()> {
        validate_plan(
            plan,
            max_actions,
            &super::super::policy::NavigationPolicy::plan_default(),
        )
    }
    #[test]
    fn validates_entire_plan_before_any_action() {
        assert!(
            validate(
                &plan(vec![
                    action("click", json!({})),
                    action("custom_js", json!({}))
                ]),
                10
            )
            .is_err()
        );
        assert!(validate(&plan(vec![action("wait", json!({"duration_ms":-1}))]), 10).is_err());
        assert!(validate(&plan(vec![action("type", json!({}))]), 10).is_err());
        assert!(validate(&plan(vec![action("click", json!({}))]), 0).is_err());
        assert!(validate(&plan(vec![action("fill", json!({"text":"hello"}))]), 10).is_ok());
        assert!(validate(&plan(vec![action("press", json!({"key":"bad-key"}))]), 10).is_err());
        assert!(
            validate(
                &plan(vec![action(
                    "press",
                    json!({"key":"Enter","modifiers":["unknown"]})
                )]),
                10
            )
            .is_err()
        );
        assert!(
            validate(
                &plan(vec![action(
                    "navigate",
                    json!({"url":"javascript:alert(1)"})
                )]),
                10
            )
            .is_err()
        );
        assert!(
            validate(
                &plan(vec![action("click", json!({"button":"invalid"}))]),
                10
            )
            .is_err()
        );
    }
    #[test]
    fn plan_navigation_follows_the_session_policy() {
        let navigate = |url: &str| plan(vec![action("navigate", json!({ "url": url }))]);
        assert!(validate(&navigate("https://example.com"), 10).is_ok());
        assert!(validate(&navigate("file:///etc/passwd"), 10).is_err());
        let policy = super::super::policy::NavigationPolicy {
            allowed_schemes: vec!["https".into()],
            allowed_domains: vec!["*.example.com".into()],
            blocked_domains: vec![],
            block_private_networks: true,
        };
        assert!(validate_plan(&navigate("https://www.example.com"), 10, &policy).is_ok());
        assert!(validate_plan(&navigate("https://example.org"), 10, &policy).is_err());
        assert!(validate_plan(&navigate("http://www.example.com"), 10, &policy).is_err());
    }
}
