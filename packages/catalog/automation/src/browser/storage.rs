use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BrowserGetLocalStorageNode {}

impl BrowserGetLocalStorageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetLocalStorageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_get_local_storage",
            "Get Local Storage",
            "Gets a value from browser localStorage",
            "Automation/Browser/Storage",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(9)
                .set_governance(5)
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

        node.add_input_pin(
            "key",
            "Key",
            "Storage key to retrieve",
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

        node.add_output_pin(
            "value",
            "Value",
            "Storage value (null if not found)",
            VariableType::String,
        );
        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the key exists",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let key: String = context.evaluate_pin("key").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let script = format!(
            "return localStorage.getItem('{}');",
            key.replace('\'', "\\'")
        );

        let result = driver
            .execute(&script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get localStorage: {}", e))?;

        let value = result.json();
        let (value_str, exists) = if value.is_null() {
            ("".to_string(), false)
        } else {
            (value.as_str().unwrap_or("").to_string(), true)
        };

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("value", json!(value_str)).await?;
        context.set_pin_value("exists", json!(exists)).await?;
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
pub struct BrowserSetLocalStorageNode {}

impl BrowserSetLocalStorageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserSetLocalStorageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_set_local_storage",
            "Set Local Storage",
            "Sets a value in browser localStorage",
            "Automation/Browser/Storage",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(9)
                .set_governance(5)
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

        node.add_input_pin("key", "Key", "Storage key", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_input_pin("value", "Value", "Value to store", VariableType::String)
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
        let key: String = context.evaluate_pin("key").await?;
        let value: String = context.evaluate_pin("value").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let script = format!(
            "localStorage.setItem('{}', '{}');",
            key.replace('\'', "\\'"),
            value.replace('\'', "\\'")
        );

        driver
            .execute(&script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to set localStorage: {}", e))?;

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
pub struct BrowserGetSessionStorageNode {}

impl BrowserGetSessionStorageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetSessionStorageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_get_session_storage",
            "Get Session Storage",
            "Gets a value from browser sessionStorage",
            "Automation/Browser/Storage",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(9)
                .set_governance(5)
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

        node.add_input_pin(
            "key",
            "Key",
            "Storage key to retrieve",
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

        node.add_output_pin(
            "value",
            "Value",
            "Storage value (null if not found)",
            VariableType::String,
        );
        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the key exists",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let key: String = context.evaluate_pin("key").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let script = format!(
            "return sessionStorage.getItem('{}');",
            key.replace('\'', "\\'")
        );

        let result = driver
            .execute(&script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get sessionStorage: {}", e))?;

        let value = result.json();
        let (value_str, exists) = if value.is_null() {
            ("".to_string(), false)
        } else {
            (value.as_str().unwrap_or("").to_string(), true)
        };

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("value", json!(value_str)).await?;
        context.set_pin_value("exists", json!(exists)).await?;
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
pub struct BrowserSetSessionStorageNode {}

impl BrowserSetSessionStorageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserSetSessionStorageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_set_session_storage",
            "Set Session Storage",
            "Sets a value in browser sessionStorage",
            "Automation/Browser/Storage",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(9)
                .set_governance(5)
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

        node.add_input_pin("key", "Key", "Storage key", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_input_pin("value", "Value", "Value to store", VariableType::String)
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
        let key: String = context.evaluate_pin("key").await?;
        let value: String = context.evaluate_pin("value").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let script = format!(
            "sessionStorage.setItem('{}', '{}');",
            key.replace('\'', "\\'"),
            value.replace('\'', "\\'")
        );

        driver
            .execute(&script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to set sessionStorage: {}", e))?;

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
pub struct BrowserClearStorageNode {}

impl BrowserClearStorageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserClearStorageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_clear_storage",
            "Clear Storage",
            "Clears localStorage and/or sessionStorage",
            "Automation/Browser/Storage",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(7)
                .set_security(7)
                .set_performance(9)
                .set_governance(7)
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

        node.add_input_pin(
            "clear_local",
            "Clear Local Storage",
            "Clear localStorage",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "clear_session",
            "Clear Session Storage",
            "Clear sessionStorage",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

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
        let clear_local: bool = context.evaluate_pin("clear_local").await?;
        let clear_session: bool = context.evaluate_pin("clear_session").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let mut script = String::new();
        if clear_local {
            script.push_str("localStorage.clear();");
        }
        if clear_session {
            script.push_str("sessionStorage.clear();");
        }

        if !script.is_empty() {
            driver
                .execute(&script, vec![])
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to clear storage: {}", e))?;
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

#[crate::register_node]
#[derive(Default)]
pub struct BrowserGetAllStorageNode {}

impl BrowserGetAllStorageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserGetAllStorageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_get_all_storage",
            "Get All Storage",
            "Gets all key-value pairs from localStorage or sessionStorage",
            "Automation/Browser/Storage",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(8)
                .set_governance(5)
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

        node.add_input_pin(
            "storage_type",
            "Storage Type",
            "Which storage to retrieve",
            VariableType::String,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(vec!["local".to_string(), "session".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("local")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "data",
            "Data",
            "All storage data as JSON object",
            VariableType::Struct,
        );

        node.add_output_pin(
            "count",
            "Count",
            "Number of items in storage",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let storage_type: String = context.evaluate_pin("storage_type").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let storage_name = if storage_type == "session" {
            "sessionStorage"
        } else {
            "localStorage"
        };

        let script = format!(
            r#"
            var data = {{}};
            for (var i = 0; i < {storage_name}.length; i++) {{
                var key = {storage_name}.key(i);
                data[key] = {storage_name}.getItem(key);
            }}
            return data;
            "#,
            storage_name = storage_name
        );

        let result = driver
            .execute(&script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get all storage: {}", e))?;

        let data = result.json();
        let count = if let Value::Object(obj) = &data {
            obj.len() as i64
        } else {
            0
        };

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("data", data.clone()).await?;
        context.set_pin_value("count", json!(count)).await?;
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
