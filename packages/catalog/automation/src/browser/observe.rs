use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ConsoleMessage {
    pub level: String,
    pub text: String,
    pub timestamp: i64,
    pub source: Option<String>,
    pub line_number: Option<i32>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct NetworkRequest {
    pub url: String,
    pub method: String,
    pub status: Option<i32>,
    pub status_text: Option<String>,
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
    pub duration_ms: Option<i64>,
    pub size_bytes: Option<i64>,
    pub resource_type: Option<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserGetConsoleLogsNode {}

impl BrowserGetConsoleLogsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetConsoleLogsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_get_console_logs",
            "Get Console Logs",
            "Retrieves console messages from the browser (logs, warnings, errors)",
            "Automation/Browser/Observe",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "level_filter",
            "Level Filter",
            "Filter by log level (empty for all)",
            VariableType::String,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(vec![
                    "".to_string(),
                    "log".to_string(),
                    "info".to_string(),
                    "warn".to_string(),
                    "error".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "logs",
            "Logs",
            "Array of console messages",
            VariableType::Struct,
        )
        .set_schema::<ConsoleMessage>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "count",
            "Count",
            "Number of log entries",
            VariableType::Integer,
        );
        node.add_output_pin(
            "has_errors",
            "Has Errors",
            "Whether there are error-level logs",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let level_filter: String = context.evaluate_pin("level_filter").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        // Inject console capture script if not already present
        let script = r#"
            if (!window.__flowlike_console_logs) {
                window.__flowlike_console_logs = [];
                const originalConsole = {};
                ['log', 'info', 'warn', 'error'].forEach(level => {
                    originalConsole[level] = console[level];
                    console[level] = function(...args) {
                        window.__flowlike_console_logs.push({
                            level: level,
                            text: args.map(a => String(a)).join(' '),
                            timestamp: Date.now(),
                            source: null,
                            line_number: null
                        });
                        originalConsole[level].apply(console, args);
                    };
                });
            }
            return window.__flowlike_console_logs || [];
        "#;

        let result = driver
            .execute(script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get console logs: {}", e))?;

        let all_logs: Vec<ConsoleMessage> =
            flow_like_types::json::from_value(result.json().clone()).unwrap_or_default();

        let logs: Vec<ConsoleMessage> = if level_filter.is_empty() {
            all_logs
        } else {
            all_logs
                .into_iter()
                .filter(|l| l.level == level_filter)
                .collect()
        };

        let has_errors = logs.iter().any(|l| l.level == "error");

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("logs", json!(logs)).await?;
        context
            .set_pin_value("count", json!(logs.len() as i64))
            .await?;
        context
            .set_pin_value("has_errors", json!(has_errors))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserClearConsoleLogsNode {}

impl BrowserClearConsoleLogsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserClearConsoleLogsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_clear_console_logs",
            "Clear Console Logs",
            "Clears the captured console log buffer",
            "Automation/Browser/Observe",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(10)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        driver
            .execute("window.__flowlike_console_logs = [];", vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to clear console logs: {}", e))?;

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserStartNetworkObserverNode {}

impl BrowserStartNetworkObserverNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserStartNetworkObserverNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_start_network_observer",
            "Start Network Observer",
            "Starts observing network requests using the Performance API",
            "Automation/Browser/Observe",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "url_pattern",
            "URL Pattern",
            "Filter requests by URL pattern (empty for all)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let url_pattern: String = context.evaluate_pin("url_pattern").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let script = format!(
            r#"
            window.__flowlike_network_requests = [];
            window.__flowlike_network_pattern = '{}';

            // Use PerformanceObserver for network entries
            if (window.PerformanceObserver) {{
                const observer = new PerformanceObserver((list) => {{
                    for (const entry of list.getEntries()) {{
                        if (entry.entryType === 'resource') {{
                            const pattern = window.__flowlike_network_pattern;
                            if (!pattern || entry.name.includes(pattern)) {{
                                window.__flowlike_network_requests.push({{
                                    url: entry.name,
                                    method: 'GET',
                                    status: null,
                                    status_text: null,
                                    request_headers: null,
                                    response_headers: null,
                                    duration_ms: Math.round(entry.duration),
                                    size_bytes: entry.transferSize || null,
                                    resource_type: entry.initiatorType
                                }});
                            }}
                        }}
                    }}
                }});
                observer.observe({{ entryTypes: ['resource'] }});
                window.__flowlike_network_observer = observer;
            }}
            "#,
            url_pattern.replace('\'', "\\'")
        );

        driver
            .execute(&script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to start network observer: {}", e))?;

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserGetNetworkRequestsNode {}

impl BrowserGetNetworkRequestsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetNetworkRequestsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_get_network_requests",
            "Get Network Requests",
            "Retrieves captured network requests from the observer",
            "Automation/Browser/Observe",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "clear_after",
            "Clear After",
            "Clear the request buffer after retrieval",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "requests",
            "Requests",
            "Array of captured network requests",
            VariableType::Struct,
        )
        .set_schema::<NetworkRequest>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "count",
            "Count",
            "Number of captured requests",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let clear_after: bool = context.evaluate_pin("clear_after").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let script = if clear_after {
            r#"
            const requests = window.__flowlike_network_requests || [];
            window.__flowlike_network_requests = [];
            return requests;
            "#
        } else {
            "return window.__flowlike_network_requests || [];"
        };

        let result = driver
            .execute(script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get network requests: {}", e))?;

        let requests: Vec<NetworkRequest> =
            flow_like_types::json::from_value(result.json().clone()).unwrap_or_default();

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("requests", json!(requests)).await?;
        context
            .set_pin_value("count", json!(requests.len() as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserWaitForNetworkIdleNode {}

impl BrowserWaitForNetworkIdleNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserWaitForNetworkIdleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_wait_for_network_idle",
            "Wait For Network Idle",
            "Waits until no network requests are in progress for a specified duration",
            "Automation/Browser/Observe",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(5)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "idle_time_ms",
            "Idle Time (ms)",
            "How long network must be idle before continuing",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(500)));

        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum time to wait for network idle",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30000)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "Triggered if timeout reached",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::time::{Duration, Instant};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let idle_time_ms: i64 = context.evaluate_pin("idle_time_ms").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms.max(0) as u64);
        let idle_duration = Duration::from_millis(idle_time_ms.max(0) as u64);
        let mut last_request_count = 0i64;
        let mut idle_start: Option<Instant> = None;

        loop {
            if start.elapsed() > timeout {
                context.set_pin_value("session_out", json!(session)).await?;
                context.activate_exec_pin("exec_timeout").await?;
                return Ok(());
            }

            let script = r#"
                return performance.getEntriesByType('resource').length;
            "#;

            let result = driver
                .execute(script, vec![])
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to check network status: {}", e))?;

            let current_count = result.json().as_i64().unwrap_or(0);

            if current_count == last_request_count {
                if idle_start.is_none() {
                    idle_start = Some(Instant::now());
                } else if idle_start.unwrap().elapsed() >= idle_duration {
                    break;
                }
            } else {
                idle_start = None;
                last_request_count = current_count;
            }

            flow_like_types::tokio::time::sleep(Duration::from_millis(100)).await;
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}
