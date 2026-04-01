use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct DomSnapshot {
    pub html: String,
    pub title: String,
    pub url: String,
    pub viewport_width: i32,
    pub viewport_height: i32,
    pub scroll_x: i32,
    pub scroll_y: i32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct AccessibilityTreeSnapshot {
    pub role: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
    pub focused: bool,
    pub children: Vec<AccessibilityTreeSnapshot>,
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserGetDomSnapshotNode {}

impl BrowserGetDomSnapshotNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetDomSnapshotNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_get_dom_snapshot",
            "Get DOM Snapshot",
            "Captures the current DOM state including HTML, title, URL, and viewport info",
            "Automation/Browser/Snapshot",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(8)
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
            "include_styles",
            "Include Styles",
            "Include computed styles (increases snapshot size)",
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
            "snapshot",
            "Snapshot",
            "DOM snapshot data",
            VariableType::Struct,
        )
        .set_schema::<DomSnapshot>();

        node.add_output_pin("html", "HTML", "Page HTML content", VariableType::String);
        node.add_output_pin("title", "Title", "Page title", VariableType::String);
        node.add_output_pin("url", "URL", "Current page URL", VariableType::String);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let _include_styles: bool = context.evaluate_pin("include_styles").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let script = r#"
            return {
                html: document.documentElement.outerHTML,
                title: document.title,
                url: window.location.href,
                viewport_width: window.innerWidth,
                viewport_height: window.innerHeight,
                scroll_x: window.scrollX,
                scroll_y: window.scrollY
            };
        "#;

        let result = driver
            .execute(script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get DOM snapshot: {}", e))?;

        let json_val = result.json();
        let snapshot = DomSnapshot {
            html: json_val["html"].as_str().unwrap_or("").to_string(),
            title: json_val["title"].as_str().unwrap_or("").to_string(),
            url: json_val["url"].as_str().unwrap_or("").to_string(),
            viewport_width: json_val["viewport_width"].as_i64().unwrap_or(0) as i32,
            viewport_height: json_val["viewport_height"].as_i64().unwrap_or(0) as i32,
            scroll_x: json_val["scroll_x"].as_i64().unwrap_or(0) as i32,
            scroll_y: json_val["scroll_y"].as_i64().unwrap_or(0) as i32,
        };

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("snapshot", json!(snapshot)).await?;
        context.set_pin_value("html", json!(snapshot.html)).await?;
        context
            .set_pin_value("title", json!(snapshot.title))
            .await?;
        context.set_pin_value("url", json!(snapshot.url)).await?;
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
pub struct BrowserGetAccessibilitySnapshotNode {}

impl BrowserGetAccessibilitySnapshotNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetAccessibilitySnapshotNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_get_accessibility_snapshot",
            "Get Accessibility Snapshot",
            "Captures the accessibility tree of the current page for screen reader analysis",
            "Automation/Browser/Snapshot",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(5)
                .set_governance(6)
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
            "max_depth",
            "Max Depth",
            "Maximum tree depth (-1 for unlimited)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_input_pin(
            "include_hidden",
            "Include Hidden",
            "Include hidden elements in the tree",
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
            "tree",
            "Tree",
            "Accessibility tree root node",
            VariableType::Struct,
        )
        .set_schema::<AccessibilityTreeSnapshot>();

        node.add_output_pin(
            "tree_json",
            "Tree JSON",
            "Accessibility tree as JSON string for LLM processing",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let max_depth: i64 = context.evaluate_pin("max_depth").await?;
        let include_hidden: bool = context.evaluate_pin("include_hidden").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        // JavaScript function to build accessibility tree
        let script = format!(
            r#"
            function buildA11yTree(element, depth, maxDepth, includeHidden) {{
                if (maxDepth !== -1 && depth > maxDepth) return null;

                const style = window.getComputedStyle(element);
                if (!includeHidden && (style.display === 'none' || style.visibility === 'hidden')) {{
                    return null;
                }}

                const role = element.getAttribute('role') || element.tagName.toLowerCase();
                const name = element.getAttribute('aria-label') ||
                             element.getAttribute('alt') ||
                             element.getAttribute('title') ||
                             (element.tagName === 'INPUT' ? element.placeholder : null) ||
                             (element.textContent && element.textContent.trim().slice(0, 100));
                const value = element.value || element.getAttribute('aria-valuenow');
                const description = element.getAttribute('aria-description');
                const focused = document.activeElement === element;

                const children = [];
                for (const child of element.children) {{
                    const childTree = buildA11yTree(child, depth + 1, maxDepth, includeHidden);
                    if (childTree) children.push(childTree);
                }}

                return {{
                    role: role,
                    name: name,
                    value: value,
                    description: description,
                    focused: focused,
                    children: children
                }};
            }}
            return buildA11yTree(document.body, 0, {max_depth}, {include_hidden});
            "#,
            max_depth = max_depth,
            include_hidden = if include_hidden { "true" } else { "false" }
        );

        let result = driver
            .execute(&script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get accessibility snapshot: {}", e))?;

        let tree: AccessibilityTreeSnapshot =
            flow_like_types::json::from_value(result.json().clone()).unwrap_or_default();

        let tree_json =
            flow_like_types::json::to_string_pretty(&tree).unwrap_or_else(|_| "{}".to_string());

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("tree", json!(tree)).await?;
        context.set_pin_value("tree_json", json!(tree_json)).await?;
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
pub struct BrowserGetElementSnapshotNode {}

impl BrowserGetElementSnapshotNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetElementSnapshotNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_get_element_snapshot",
            "Get Element Snapshot",
            "Gets detailed information about a specific element by selector",
            "Automation/Browser/Snapshot",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(5)
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
            "selector",
            "Selector",
            "CSS selector of element",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_not_found",
            "Not Found",
            "Triggered if element not found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("html", "HTML", "Element outer HTML", VariableType::String);
        node.add_output_pin("text", "Text", "Element text content", VariableType::String);
        node.add_output_pin("tag", "Tag", "Element tag name", VariableType::String);
        node.add_output_pin("x", "X", "Element X position", VariableType::Integer);
        node.add_output_pin("y", "Y", "Element Y position", VariableType::Integer);
        node.add_output_pin("width", "Width", "Element width", VariableType::Integer);
        node.add_output_pin("height", "Height", "Element height", VariableType::Integer);
        node.add_output_pin(
            "visible",
            "Visible",
            "Whether element is visible",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::By;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_not_found").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let element = match driver.find(By::Css(&selector)).await {
            Ok(el) => el,
            Err(_) => {
                context.set_pin_value("session_out", json!(session)).await?;
                context.activate_exec_pin("exec_not_found").await?;
                return Ok(());
            }
        };

        let html = element.outer_html().await.unwrap_or_default();
        let text = element.text().await.unwrap_or_default();
        let tag = element.tag_name().await.unwrap_or_default();
        let rect = element.rect().await.ok();
        let displayed = element.is_displayed().await.unwrap_or(false);

        let (x, y, width, height) = rect
            .map(|r| (r.x as i64, r.y as i64, r.width as i64, r.height as i64))
            .unwrap_or((0, 0, 0, 0));

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("html", json!(html)).await?;
        context.set_pin_value("text", json!(text)).await?;
        context.set_pin_value("tag", json!(tag)).await?;
        context.set_pin_value("x", json!(x)).await?;
        context.set_pin_value("y", json!(y)).await?;
        context.set_pin_value("width", json!(width)).await?;
        context.set_pin_value("height", json!(height)).await?;
        context.set_pin_value("visible", json!(displayed)).await?;
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
