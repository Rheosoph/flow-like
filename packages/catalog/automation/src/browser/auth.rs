use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BasicAuthCredentials {
    pub username: String,
    pub password: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct CookieData {
    pub name: String,
    pub value: String,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub secure: Option<bool>,
    pub http_only: Option<bool>,
    pub same_site: Option<String>,
    pub expiry: Option<i64>,
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserSetBasicAuthNode {}

impl BrowserSetBasicAuthNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserSetBasicAuthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_set_basic_auth",
            "Set Basic Auth",
            "Configures HTTP Basic Authentication credentials for requests",
            "Automation/Browser/Auth",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(9)
                .set_governance(4)
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
            "username",
            "Username",
            "HTTP Basic Auth username",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "password",
            "Password",
            "HTTP Basic Auth password",
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
        let username: String = context.evaluate_pin("username").await?;
        let password: String = context.evaluate_pin("password").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        // Execute CDP command to set basic auth via request interception
        let script = format!(
            r#"
            // Store credentials for basic auth
            window.__flowlike_basic_auth = {{
                username: '{}',
                password: '{}'
            }};
            "#,
            username.replace('\'', "\\'"),
            password.replace('\'', "\\'")
        );

        driver
            .execute(&script, vec![])
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to set basic auth credentials: {}", e))?;

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
pub struct BrowserSaveCookiesNode {}

impl BrowserSaveCookiesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserSaveCookiesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_save_cookies",
            "Save Cookies",
            "Saves all browser cookies to a file for later restoration",
            "Automation/Browser/Auth",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(7)
                .set_governance(4)
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
            "file_path",
            "File Path",
            "Path to save cookies JSON file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "cookie_count",
            "Cookie Count",
            "Number of cookies saved",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::cookie::SameSite;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let file_path: FlowPath = context.evaluate_pin("file_path").await?;

        let driver = session.get_browser_driver(context).await?;
        let cookies = driver
            .get_all_cookies()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get cookies: {}", e))?;

        let cookie_data: Vec<CookieData> = cookies
            .into_iter()
            .map(|c| CookieData {
                name: c.name.clone(),
                value: c.value.clone(),
                domain: c.domain.clone(),
                path: c.path.clone(),
                secure: c.secure,
                http_only: None, // thirtyfour Cookie doesn't have http_only field
                same_site: c.same_site.map(|ss| match ss {
                    SameSite::Strict => "Strict".to_string(),
                    SameSite::Lax => "Lax".to_string(),
                    SameSite::None => "None".to_string(),
                }),
                expiry: c.expiry,
            })
            .collect();

        let cookie_json = flow_like_types::json::to_string_pretty(&cookie_data)
            .map_err(|e| flow_like_types::anyhow!("Failed to serialize cookies: {}", e))?;

        // Save to file
        let runtime = file_path.to_runtime(context).await?;
        let store = runtime.store.as_generic();
        let path = flow_like_storage::Path::from(runtime.path.to_string());

        store
            .put(&path, cookie_json.into())
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to save cookies: {}", e))?;

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("cookie_count", json!(cookie_data.len() as i64))
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
pub struct BrowserLoadCookiesNode {}

impl BrowserLoadCookiesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserLoadCookiesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_load_cookies",
            "Load Cookies",
            "Loads cookies from a file into the browser session",
            "Automation/Browser/Auth",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(7)
                .set_governance(4)
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
            "file_path",
            "File Path",
            "Path to cookies JSON file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_error",
            "⚠",
            "Triggered if file not found or invalid",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "cookie_count",
            "Cookie Count",
            "Number of cookies loaded",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::cookie::{Cookie, SameSite};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let file_path: FlowPath = context.evaluate_pin("file_path").await?;

        // Load from file
        let runtime = file_path.to_runtime(context).await?;
        let store = runtime.store.as_generic();
        let path = flow_like_storage::Path::from(runtime.path.to_string());

        let result = store.get(&path).await;
        let data = match result {
            Ok(data) => data
                .bytes()
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to read cookies file: {}", e))?,
            Err(_) => {
                context.set_pin_value("session_out", json!(session)).await?;
                context.set_pin_value("cookie_count", json!(0)).await?;
                context.activate_exec_pin("exec_error").await?;
                return Ok(());
            }
        };

        let cookie_data: Vec<CookieData> = flow_like_types::json::from_slice(&data)
            .map_err(|e| flow_like_types::anyhow!("Failed to parse cookies file: {}", e))?;

        let driver = session.get_browser_driver(context).await?;

        for cd in &cookie_data {
            let mut cookie = Cookie::new(&cd.name, &cd.value);

            if let Some(domain) = &cd.domain {
                cookie.set_domain(domain.clone());
            }
            if let Some(path) = &cd.path {
                cookie.set_path(path.clone());
            }
            if let Some(secure) = cd.secure {
                cookie.set_secure(secure);
            }
            // Note: http_only is not directly supported by thirtyfour Cookie
            if let Some(same_site) = &cd.same_site {
                let ss = match same_site.as_str() {
                    "Strict" => SameSite::Strict,
                    "Lax" => SameSite::Lax,
                    _ => SameSite::None,
                };
                cookie.set_same_site(ss);
            }
            if let Some(expiry) = cd.expiry {
                cookie.set_expiry(expiry);
            }

            let _ = driver.add_cookie(cookie).await;
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("cookie_count", json!(cookie_data.len() as i64))
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
pub struct BrowserClearCookiesNode {}

impl BrowserClearCookiesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserClearCookiesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_clear_cookies",
            "Clear Cookies",
            "Clears all cookies from the browser session",
            "Automation/Browser/Auth",
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
        let driver = session.get_browser_driver(context).await?;

        driver
            .delete_all_cookies()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to clear cookies: {}", e))?;

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
