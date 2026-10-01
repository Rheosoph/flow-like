#[cfg(feature = "execute")]
use super::driver::PageContext;
use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_browser::script::{ScriptArg, ScriptValue};
#[cfg(any(feature = "execute", test))]
use flow_like_types::Value;
use flow_like_types::{async_trait, json::json};

/// Keys and values travel as script arguments, never as script text.
#[cfg(any(feature = "execute", test))]
const STORAGE_PRELUDE: &str =
    "const storage = arguments[0] === 'session' ? window.sessionStorage : window.localStorage;";
#[cfg(any(feature = "execute", test))]
const GET_ITEM_SCRIPT: &str = "return storage.getItem(arguments[1]);";
#[cfg(any(feature = "execute", test))]
const SET_ITEM_SCRIPT: &str = "storage.setItem(arguments[1], arguments[2]);";
#[cfg(any(feature = "execute", test))]
const ALL_ITEMS_SCRIPT: &str = "return Array.from({ length: storage.length }, (_, index) => { const key = storage.key(index); return [key, storage.getItem(key)]; });";

#[cfg(any(feature = "execute", test))]
#[derive(Clone, Copy, Debug, PartialEq)]
enum StorageArea {
    Local,
    Session,
}

#[cfg(any(feature = "execute", test))]
impl StorageArea {
    fn argument(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Session => "session",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Local => "localStorage",
            Self::Session => "sessionStorage",
        }
    }
}

#[cfg(any(feature = "execute", test))]
fn storage_script(body: &str) -> String {
    format!("{STORAGE_PRELUDE} {body}")
}

#[cfg(any(feature = "execute", test))]
fn storage_arguments(area: StorageArea, key: Option<&str>, value: Option<&str>) -> Vec<Value> {
    let mut arguments = vec![json!(area.argument())];
    arguments.extend(key.map(|key| json!(key)));
    arguments.extend(value.map(|value| json!(value)));
    arguments
}

#[cfg(any(feature = "execute", test))]
fn storage_entries(
    value: &Value,
) -> flow_like_types::Result<flow_like_types::json::Map<String, Value>> {
    let entries = value.as_array().ok_or_else(|| {
        flow_like_types::anyhow!("Browser storage listing returned a non-array value: {value}")
    })?;
    entries
        .iter()
        .map(|entry| match entry.as_array().map(Vec::as_slice) {
            Some([Value::String(key), item]) => Ok((key.clone(), item.clone())),
            _ => Err(flow_like_types::anyhow!(
                "Browser storage listing returned a malformed entry: {entry}"
            )),
        })
        .collect()
}

#[cfg(feature = "execute")]
fn script_arguments(area: StorageArea, key: Option<&str>, value: Option<&str>) -> Vec<ScriptArg> {
    storage_arguments(area, key, value)
        .into_iter()
        .map(ScriptArg::Json)
        .collect()
}

/// Reads have no side effects, so they run as probes that a navigation retries, as chromedriver
/// re-ran every script; writes run as user scripts, which are never retried.
#[cfg(feature = "execute")]
async fn read_storage(
    page: &PageContext,
    body: &str,
    area: StorageArea,
    key: Option<&str>,
) -> flow_like_types::Result<ScriptValue> {
    page.probe(&storage_script(body), script_arguments(area, key, None))
        .await
}

#[cfg(feature = "execute")]
async fn get_item(
    page: &PageContext,
    area: StorageArea,
    key: &str,
) -> flow_like_types::Result<Option<String>> {
    let result = read_storage(page, GET_ITEM_SCRIPT, area, Some(key))
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to read {} key '{key}': {e}", area.name()))?;
    Ok(result.json().as_str().map(str::to_owned))
}

#[cfg(feature = "execute")]
async fn set_item(
    page: &PageContext,
    area: StorageArea,
    key: &str,
    value: &str,
) -> flow_like_types::Result<()> {
    let arguments = script_arguments(area, Some(key), Some(value));
    page.execute(&storage_script(SET_ITEM_SCRIPT), arguments)
        .await
        .map_err(|e| {
            flow_like_types::anyhow!("Failed to write {} key '{key}': {e}", area.name())
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_payload_keeps_keys_and_values_out_of_the_script() {
        let key = "C:\\temp\\new';alert(1);//";
        let value = "line one\nline two\r\n\u{2028}\"quoted\"";
        let arguments = storage_arguments(StorageArea::Session, Some(key), Some(value));
        assert_eq!(arguments, vec![json!("session"), json!(key), json!(value)]);
        for body in [GET_ITEM_SCRIPT, SET_ITEM_SCRIPT, ALL_ITEMS_SCRIPT] {
            let script = storage_script(body);
            assert!(!script.contains(key) && !script.contains("line one"));
            assert!(script.starts_with(STORAGE_PRELUDE));
        }
        assert_eq!(
            storage_arguments(StorageArea::Local, None, None),
            vec![json!("local")]
        );
    }

    #[test]
    fn storage_listing_keeps_prototype_like_keys_as_data() {
        let entries =
            storage_entries(&json!([["__proto__", "x"], ["a\\b", null], ["", ""]])).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries["__proto__"], json!("x"));
        assert_eq!(entries["a\\b"], Value::Null);
        assert!(storage_entries(&json!({"a": "b"})).is_err());
        assert!(storage_entries(&json!([["only-key"]])).is_err());
    }
}

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
        node.set_flowscript_name("browser", "getLocalStorage");
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

        let page = session.browser_page(context).await?;
        let value = get_item(&page, StorageArea::Local, &key).await?;

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("exists", json!(value.is_some()))
            .await?;
        context
            .set_pin_value("value", json!(value.unwrap_or_default()))
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
        node.set_flowscript_name("browser", "setLocalStorage");
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

        let page = session.browser_page(context).await?;
        set_item(&page, StorageArea::Local, &key, &value).await?;

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
        node.set_flowscript_name("browser", "getSessionStorage");
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

        let page = session.browser_page(context).await?;
        let value = get_item(&page, StorageArea::Session, &key).await?;

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("exists", json!(value.is_some()))
            .await?;
        context
            .set_pin_value("value", json!(value.unwrap_or_default()))
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
        node.set_flowscript_name("browser", "setSessionStorage");
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

        let page = session.browser_page(context).await?;
        set_item(&page, StorageArea::Session, &key, &value).await?;

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
        node.set_flowscript_name("browser", "clearStorage");
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

        let page = session.browser_page(context).await?;

        let mut script = String::new();
        if clear_local {
            script.push_str("localStorage.clear();");
        }
        if clear_session {
            script.push_str("sessionStorage.clear();");
        }

        if !script.is_empty() {
            page.execute(&script, Vec::new())
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
        node.set_flowscript_name("browser", "getAllStorage");
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
        )
        .set_open_schema();

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

        let page = session.browser_page(context).await?;

        let area = if storage_type == "session" {
            StorageArea::Session
        } else {
            StorageArea::Local
        };

        let result = read_storage(&page, ALL_ITEMS_SCRIPT, area, None)
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to list {}: {e}", area.name()))?;
        let data = storage_entries(result.json())?;
        let count = data.len() as i64;

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("data", Value::Object(data)).await?;
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
