use super::content::page_node;
#[cfg(feature = "execute")]
use crate::types::handles::AutomationSession;
#[cfg(feature = "execute")]
use crate::types::selectors::Selector;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(any(feature = "execute", test))]
use flow_like_types::Value;
use flow_like_types::{async_trait, json::json};

const CONDITIONS: [&str; 8] = [
    "text_visible",
    "text_gone",
    "url_matches",
    "title_contains",
    "load_state",
    "js_truthy",
    "element_visible",
    "element_hidden",
];

#[cfg(any(feature = "execute", test))]
const NETWORK_IDLE_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

#[cfg(feature = "execute")]
const TEXT_VISIBLE_SCRIPT: &str = "const normalize = (value) => String(value).replace(/\\s+/g, ' ').trim(); const body = document.body; return !!body && normalize(body.innerText).includes(normalize(arguments[0]));";

/// `*` matches any run of characters (including `/`), `?` exactly one; the whole URL must match.
#[cfg(any(feature = "execute", test))]
fn glob_matches(pattern: &str, text: &str) -> bool {
    let text: Vec<char> = text.chars().collect();
    let mut matches = vec![false; text.len() + 1];
    matches[0] = true;
    for character in pattern.chars() {
        if character == '*' {
            for index in 1..=text.len() {
                matches[index] |= matches[index - 1];
            }
        } else {
            for index in (1..=text.len()).rev() {
                matches[index] =
                    matches[index - 1] && (character == '?' || character == text[index - 1]);
            }
            matches[0] = false;
        }
    }
    matches[text.len()]
}

#[cfg(any(feature = "execute", test))]
enum UrlPattern {
    Glob(String),
    Regex(flow_like_types::regex::Regex),
}

#[cfg(any(feature = "execute", test))]
impl UrlPattern {
    fn parse(pattern: &str) -> flow_like_types::Result<Self> {
        let pattern = pattern.trim();
        if let Some(expression) = pattern.strip_prefix("re:") {
            return flow_like_types::regex::Regex::new(expression)
                .map(Self::Regex)
                .map_err(|e| flow_like_types::anyhow!("Invalid URL regex '{expression}': {e}"));
        }
        if pattern.is_empty() {
            return Err(flow_like_types::anyhow!(
                "url_matches needs a glob such as https://example.com/* or a re: regex"
            ));
        }
        Ok(Self::Glob(pattern.to_owned()))
    }

    fn matches(&self, url: &str) -> bool {
        match self {
            Self::Glob(pattern) => glob_matches(pattern, url),
            Self::Regex(regex) => regex.is_match(url),
        }
    }
}

#[cfg(any(feature = "execute", test))]
#[derive(Debug, Clone, Copy, PartialEq)]
enum LoadState {
    DomContentLoaded,
    Load,
    NetworkIdle,
}

#[cfg(any(feature = "execute", test))]
impl LoadState {
    fn parse(value: &str) -> flow_like_types::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "load" => Ok(Self::Load),
            "domcontentloaded" => Ok(Self::DomContentLoaded),
            "networkidle" => Ok(Self::NetworkIdle),
            other => Err(flow_like_types::anyhow!(
                "Load state must be domcontentloaded, load, or networkidle, got '{other}'"
            )),
        }
    }

    fn reached_by(self, ready_state: &str) -> bool {
        match self {
            Self::DomContentLoaded => matches!(ready_state, "interactive" | "complete"),
            Self::Load | Self::NetworkIdle => ready_state == "complete",
        }
    }
}

#[cfg(any(feature = "execute", test))]
enum Condition {
    TextVisible(String),
    TextGone(String),
    UrlMatches(UrlPattern),
    TitleContains(String),
    LoadState(LoadState),
    JsTruthy(String),
    ElementVisible,
    ElementHidden,
}

#[cfg(any(feature = "execute", test))]
impl Condition {
    fn parse(condition: &str, value: &str) -> flow_like_types::Result<Self> {
        let required = |what: &str| {
            if value.trim().is_empty() {
                Err(flow_like_types::anyhow!(
                    "{condition} needs {what} in Value"
                ))
            } else {
                Ok(value.to_owned())
            }
        };
        Ok(match condition {
            "text_visible" => Self::TextVisible(required("the text to find")?),
            "text_gone" => Self::TextGone(required("the text to wait out")?),
            "url_matches" => Self::UrlMatches(UrlPattern::parse(value)?),
            "title_contains" => Self::TitleContains(required("part of the title")?),
            "load_state" => Self::LoadState(LoadState::parse(value)?),
            "js_truthy" => Self::JsTruthy(required("a JavaScript function body")?),
            "element_visible" => Self::ElementVisible,
            "element_hidden" => Self::ElementHidden,
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unknown wait condition '{other}'; use one of {}",
                    CONDITIONS.join(", ")
                ));
            }
        })
    }

    fn targets_element(&self) -> bool {
        matches!(self, Self::ElementVisible | Self::ElementHidden)
    }
}

/// JavaScript truthiness of a WebDriver script result (`undefined` arrives as null).
#[cfg(any(feature = "execute", test))]
fn json_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

#[cfg(feature = "execute")]
fn is_webdriver_error(
    error: &flow_like_types::Error,
    matches: impl Fn(&thirtyfour::error::WebDriverErrorInner) -> bool,
) -> bool {
    error
        .downcast_ref::<thirtyfour::error::WebDriverError>()
        .is_some_and(|error| matches(error.as_inner()))
}

#[cfg(feature = "execute")]
async fn element_displayed(
    driver: &thirtyfour::WebDriver,
    locator: &Selector,
) -> flow_like_types::Result<bool> {
    use thirtyfour::error::WebDriverErrorInner::{NoSuchElement, StaleElementReference};
    let element = match super::selector::find(driver, locator).await {
        Ok(element) => element,
        Err(error)
            if is_webdriver_error(&error, |inner| {
                matches!(inner, NoSuchElement(_) | StaleElementReference(_))
            }) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    match element.is_displayed().await {
        Ok(displayed) => Ok(displayed),
        Err(error) if matches!(error.as_inner(), StaleElementReference(_)) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(feature = "execute")]
struct ConditionProbe<'a> {
    driver: &'a thirtyfour::WebDriver,
    condition: &'a Condition,
    locator: &'a Selector,
    network: Option<std::sync::Arc<tokio::sync::Mutex<super::protocol::NetworkState>>>,
    last_script_error: Option<String>,
}

#[cfg(feature = "execute")]
impl ConditionProbe<'_> {
    async fn is_met(&mut self) -> flow_like_types::Result<bool> {
        let driver = self.driver;
        match self.condition {
            Condition::TextVisible(text) | Condition::TextGone(text) => {
                let visible = driver
                    .execute(TEXT_VISIBLE_SCRIPT, vec![json!(text)])
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to read page text: {e}"))?
                    .json()
                    == &json!(true);
                Ok(visible == matches!(self.condition, Condition::TextVisible(_)))
            }
            Condition::UrlMatches(pattern) => {
                Ok(pattern.matches(driver.current_url().await?.as_str()))
            }
            Condition::TitleContains(text) => Ok(driver.title().await?.contains(text.as_str())),
            Condition::LoadState(state) => {
                let ready = driver
                    .execute("return document.readyState;", vec![])
                    .await?
                    .json()
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if !state.reached_by(&ready) {
                    return Ok(false);
                }
                let Some(network) = &self.network else {
                    return Ok(true);
                };
                let network = network.lock().await;
                if let Some(failure) = &network.failure {
                    return Err(flow_like_types::anyhow!(failure.clone()));
                }
                Ok(super::protocol::has_been_idle(
                    network.pending.len(),
                    network.last_activity,
                    std::time::Instant::now(),
                    NETWORK_IDLE_WINDOW,
                ))
            }
            Condition::JsTruthy(body) => match driver.execute(body.as_str(), vec![]).await {
                Ok(result) => Ok(json_truthy(result.json())),
                Err(error)
                    if matches!(
                        error.as_inner(),
                        thirtyfour::error::WebDriverErrorInner::JavascriptError(_)
                    ) =>
                {
                    self.last_script_error = Some(error.to_string());
                    Ok(false)
                }
                Err(error) => Err(error.into()),
            },
            Condition::ElementVisible => element_displayed(driver, self.locator).await,
            Condition::ElementHidden => Ok(!element_displayed(driver, self.locator).await?),
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserWaitForConditionNode {}

impl BrowserWaitForConditionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserWaitForConditionNode {
    fn get_node(&self) -> Node {
        let mut node = page_node(
            "browser_wait_for_condition",
            "Wait For Condition",
            "Polls the current page until a condition holds, then continues on Met, or on Timeout when the deadline passes. Conditions: text_visible / text_gone (rendered page text contains Value), url_matches (glob with * and ?, or re:<regex>), title_contains, load_state (domcontentloaded, load, or networkidle — networkidle needs a running Network Observer), js_truthy (Value is a JavaScript function body whose return value is tested, e.g. return window.appReady === true), element_visible / element_hidden (CSS selector in Value, or a connected Locator).",
            "Wait",
        );
        node.set_flowscript_name("browser", "waitForCondition");
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(7)
                .set_governance(6)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        if let Some(pin) = node.get_pin_mut_by_name("exec_out") {
            pin.description = "Condition was met".into();
        }
        node.add_input_pin(
            "condition",
            "Condition",
            "What to wait for",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(CONDITIONS.map(str::to_string).to_vec())
                .build(),
        )
        .set_default_value(Some(json!("text_visible")));
        node.add_input_pin(
            "value",
            "Value",
            "Text, URL pattern, title part, load state, JavaScript function body, or CSS selector, depending on Condition",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        super::selector::add_locator_pin(&mut node);
        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum time to wait",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30000)));
        node.add_input_pin(
            "poll_ms",
            "Poll Interval (ms)",
            "Pause between checks, at least 10",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "The condition did not hold before the deadline",
            VariableType::Execution,
        );
        node.add_output_pin(
            "met",
            "Met",
            "Whether the condition held before the deadline",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "elapsed_ms",
            "Elapsed (ms)",
            "Time spent waiting",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::time::{Duration, Instant};
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_timeout").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let condition: String = context.evaluate_pin("condition").await?;
        let value: String = context.evaluate_pin("value").await?;
        let condition = Condition::parse(&condition, &value)?;
        let locator = if condition.targets_element() {
            let locator = super::selector::evaluate_locator(context, value.trim()).await?;
            if locator.value.is_empty() {
                return Err(flow_like_types::anyhow!(
                    "Element conditions need a CSS selector in Value or a connected Locator"
                ));
            }
            locator
        } else {
            Selector::css("")
        };
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;
        let poll_ms: i64 = context.evaluate_pin("poll_ms").await?;
        if timeout_ms < 0 {
            return Err(flow_like_types::anyhow!(
                "Wait timeout must be nonnegative, got {timeout_ms}"
            ));
        }
        if !(10..=60_000).contains(&poll_ms) {
            return Err(flow_like_types::anyhow!(
                "Poll interval must be between 10 and 60000 ms, got {poll_ms}"
            ));
        }
        let network = if matches!(condition, Condition::LoadState(LoadState::NetworkIdle)) {
            Some(super::protocol::network_state(context, &session).await?)
        } else {
            None
        };

        let driver = session.get_browser_driver_and_switch(context).await?;
        let mut probe = ConditionProbe {
            driver: &driver,
            condition: &condition,
            locator: &locator,
            network,
            last_script_error: None,
        };
        let timeout = Duration::from_millis(timeout_ms as u64);
        let poll = Duration::from_millis(poll_ms as u64);
        let start = Instant::now();
        let met = loop {
            context.check_cancelled()?;
            if probe.is_met().await? {
                break true;
            }
            let elapsed = start.elapsed();
            if elapsed >= timeout {
                break false;
            }
            crate::rpa::branch::delay(context, poll.min(timeout - elapsed)).await?;
        };
        let elapsed_ms = start.elapsed().as_millis().min(i64::MAX as u128) as i64;
        let last_script_error = probe.last_script_error.take();
        drop(probe);
        drop(driver);

        if let (false, Some(error)) = (met, last_script_error) {
            context.log_message(
                &format!("js_truthy condition kept throwing until the timeout: {error}"),
                flow_like::flow::execution::LogLevel::Warn,
            );
        }
        context.set_pin_value("met", json!(met)).await?;
        context
            .set_pin_value("elapsed_ms", json!(elapsed_ms))
            .await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context
            .activate_exec_pin(if met { "exec_out" } else { "exec_timeout" })
            .await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_globs_match_whole_urls() {
        let pattern = UrlPattern::parse("https://example.com/orders/*").unwrap();
        assert!(pattern.matches("https://example.com/orders/42?tab=items"));
        assert!(!pattern.matches("https://example.com/order"));
        assert!(!pattern.matches("https://evil.test/https://example.com/orders/1x"));
        assert!(
            UrlPattern::parse("*://*/login?next=*")
                .unwrap()
                .matches("http://a.b/login?next=%2F")
        );
        assert!(
            UrlPattern::parse("https://example.com/?")
                .unwrap()
                .matches("https://example.com/x")
        );
        assert!(UrlPattern::parse("  ").is_err());
    }

    #[test]
    fn url_regexes_use_the_re_prefix() {
        let pattern = UrlPattern::parse(r"re:^https://shop\.test/cart/\d+$").unwrap();
        assert!(pattern.matches("https://shop.test/cart/17"));
        assert!(!pattern.matches("https://shop.test/cart/x"));
        assert!(UrlPattern::parse("re:(unclosed").is_err());
    }

    #[test]
    fn conditions_validate_their_value() {
        assert!(Condition::parse("text_visible", "  ").is_err());
        assert!(Condition::parse("js_truthy", "").is_err());
        assert!(Condition::parse("hover", "x").is_err());
        assert!(matches!(
            Condition::parse("load_state", "").unwrap(),
            Condition::LoadState(LoadState::Load)
        ));
        assert!(Condition::parse("load_state", "idle").is_err());
        assert!(
            Condition::parse("element_hidden", "")
                .unwrap()
                .targets_element()
        );
        assert!(
            !Condition::parse("title_contains", "Inbox")
                .unwrap()
                .targets_element()
        );
    }

    #[test]
    fn load_states_follow_document_ready_state() {
        assert!(LoadState::DomContentLoaded.reached_by("interactive"));
        assert!(!LoadState::Load.reached_by("interactive"));
        assert!(LoadState::NetworkIdle.reached_by("complete"));
        assert!(!LoadState::DomContentLoaded.reached_by("loading"));
    }

    #[test]
    fn script_results_follow_javascript_truthiness() {
        for falsy in [json!(null), json!(false), json!(0), json!(0.0), json!("")] {
            assert!(!json_truthy(&falsy), "{falsy} should be falsy");
        }
        for truthy in [json!(true), json!(-1), json!("0"), json!([]), json!({})] {
            assert!(json_truthy(&truthy), "{truthy} should be truthy");
        }
    }
}
