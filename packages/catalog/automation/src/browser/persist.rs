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
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Browser storage state in Playwright's `storageState` shape.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct StorageState {
    #[serde(default)]
    pub cookies: Vec<StorageCookie>,
    #[serde(default)]
    pub origins: Vec<OriginStorage>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StorageCookie {
    pub name: String,
    pub value: String,
    /// Cookie domain; empty means the current page.
    #[serde(default)]
    pub domain: String,
    #[serde(default = "root_path")]
    pub path: String,
    /// Expiry in Unix seconds; -1 for session cookies.
    #[serde(default = "session_expiry")]
    pub expires: f64,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub secure: bool,
    /// Strict, Lax or None.
    #[serde(default = "lax")]
    pub same_site: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OriginStorage {
    pub origin: String,
    #[serde(default)]
    pub local_storage: Vec<StorageItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_storage: Vec<StorageItem>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct StorageItem {
    pub name: String,
    pub value: String,
}

fn root_path() -> String {
    "/".to_string()
}

fn session_expiry() -> f64 {
    -1.0
}

fn lax() -> String {
    "Lax".to_string()
}

#[cfg(any(feature = "execute", test))]
fn same_site(value: Option<&str>) -> String {
    match value.map(str::to_ascii_lowercase).as_deref() {
        Some("strict") => "Strict",
        Some("none") | Some("no_restriction") => "None",
        _ => "Lax",
    }
    .to_string()
}

#[cfg(any(feature = "execute", test))]
impl StorageCookie {
    /// Parses a CDP `Network.Cookie`.
    pub(crate) fn from_cdp(cookie: &flow_like_types::Value) -> Option<Self> {
        let session = cookie["session"].as_bool().unwrap_or(false);
        let expires = cookie["expires"]
            .as_f64()
            .filter(|expires| expires.is_finite() && *expires > 0.0 && !session)
            .unwrap_or(-1.0);
        Some(Self {
            name: cookie["name"].as_str()?.to_string(),
            value: cookie["value"].as_str().unwrap_or_default().to_string(),
            domain: cookie["domain"].as_str().unwrap_or_default().to_string(),
            path: cookie["path"].as_str().unwrap_or("/").to_string(),
            expires,
            http_only: cookie["httpOnly"].as_bool().unwrap_or(false),
            secure: cookie["secure"].as_bool().unwrap_or(false),
            same_site: same_site(cookie["sameSite"].as_str()),
        })
    }

    /// CDP `Network.CookieParam`; cookies without a domain are bound to `page_url`.
    pub(crate) fn to_cdp(&self, page_url: &str) -> flow_like_types::Value {
        let mut param = json!({
            "name": self.name,
            "value": self.value,
            "path": self.path,
            "secure": self.secure,
            "httpOnly": self.http_only,
            "sameSite": same_site(Some(&self.same_site)),
        });
        if self.domain.is_empty() {
            param["url"] = json!(page_url);
        } else {
            param["domain"] = json!(self.domain);
        }
        if self.expires > 0.0 && self.expires.is_finite() {
            param["expires"] = json!(self.expires);
        }
        param
    }

    pub(crate) fn expired_at(&self, now: f64) -> bool {
        self.expires > 0.0 && self.expires < now
    }
}

/// All cookies of the browser (every domain, including HttpOnly).
#[cfg(feature = "execute")]
pub(crate) async fn read_cookies(ctx: &PageContext) -> flow_like_types::Result<Vec<StorageCookie>> {
    use super::cdp::send;
    let response = match send(ctx, "Network.getAllCookies", json!({})).await {
        Ok(response) => response,
        Err(_) => send(ctx, "Storage.getCookies", json!({})).await?,
    };
    Ok(response["cookies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(StorageCookie::from_cdp)
        .collect())
}

#[cfg(feature = "execute")]
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CookieWriteReport {
    pub applied: usize,
    pub failed: usize,
}

#[cfg(feature = "execute")]
fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64())
        .unwrap_or_default()
}

/// Sets cookies for any domain (keeping HttpOnly, Secure, SameSite and expiry); cookies without
/// a domain are bound to the current page. Expired cookies count as failed.
#[cfg(feature = "execute")]
pub(crate) async fn write_cookies(
    context: &mut ExecutionContext,
    ctx: &PageContext,
    cookies: &[StorageCookie],
) -> flow_like_types::Result<CookieWriteReport> {
    use super::cdp::send;
    use flow_like::flow::execution::LogLevel;
    let now = unix_now();
    let (live, expired): (Vec<&StorageCookie>, Vec<&StorageCookie>) =
        cookies.iter().partition(|cookie| !cookie.expired_at(now));
    let mut report = CookieWriteReport {
        applied: 0,
        failed: expired.len(),
    };
    if !expired.is_empty() {
        context.log_message(
            &format!("Skipped {} expired cookies", expired.len()),
            LogLevel::Warn,
        );
    }
    if live.is_empty() {
        return Ok(report);
    }
    let page_url = ctx.page.url().await?;
    let params: Vec<_> = live.iter().map(|cookie| cookie.to_cdp(&page_url)).collect();
    if send(ctx, "Network.setCookies", json!({ "cookies": params }))
        .await
        .is_ok()
    {
        report.applied += params.len();
        return Ok(report);
    }
    for (cookie, param) in live.iter().zip(params) {
        match send(ctx, "Network.setCookie", param).await {
            Ok(result) if result["success"].as_bool() != Some(false) => report.applied += 1,
            Ok(_) | Err(_) => {
                report.failed += 1;
                context.log_message(
                    &format!(
                        "Browser rejected cookie '{}' for domain '{}'",
                        cookie.name, cookie.domain
                    ),
                    LogLevel::Warn,
                );
            }
        }
    }
    Ok(report)
}

/// Deletes every cookie of the browser.
#[cfg(feature = "execute")]
pub(crate) async fn clear_cookies(ctx: &PageContext) -> flow_like_types::Result<()> {
    super::cdp::send(ctx, "Network.clearBrowserCookies", json!({})).await?;
    Ok(())
}

#[cfg(feature = "execute")]
const READ_STORAGE: &str = r#"
const read = (area) => {
  const items = [];
  try {
    const storage = window[area];
    for (let i = 0; i < storage.length; i++) {
      const name = storage.key(i);
      items.push({ name, value: storage.getItem(name) ?? '' });
    }
  } catch (error) {}
  return items;
};
return { origin: location.origin, localStorage: read('localStorage'), sessionStorage: arguments[0] ? read('sessionStorage') : [] };
"#;

#[cfg(feature = "execute")]
const WRITE_STORAGE: &str = r#"
const [local, session] = arguments;
for (const item of local) window.localStorage.setItem(item.name, item.value);
for (const item of session) window.sessionStorage.setItem(item.name, item.value);
return true;
"#;

/// Runs a storage script in the top-level document, so it sees the origin of the page even when
/// the session is inside a frame.
#[cfg(feature = "execute")]
async fn run_top_level<T: flow_like_types::json::DeserializeOwned>(
    ctx: &PageContext,
    script: &str,
    args: Vec<flow_like_types::Value>,
) -> flow_like_types::Result<T> {
    use flow_like_browser::script::{ScriptArg, ScriptOptions};
    let args = args.into_iter().map(ScriptArg::Json).collect();
    let result = ctx
        .page
        .main_frame()
        .execute_script(script, args, ScriptOptions::USER)
        .await?;
    Ok(flow_like_types::json::from_value(result.into_json())?)
}

#[cfg(feature = "execute")]
async fn read_storage_file(
    context: &mut ExecutionContext,
    path: &FlowPath,
) -> flow_like_types::Result<Vec<u8>> {
    let runtime = path.to_runtime(context).await?;
    let data = runtime
        .store
        .as_generic()
        .get(&runtime.path)
        .await
        .map_err(|error| {
            flow_like_types::anyhow!("Failed to open storage state {}: {error}", runtime.path)
        })?
        .bytes()
        .await
        .map_err(|error| {
            flow_like_types::anyhow!("Failed to read storage state {}: {error}", runtime.path)
        })?;
    Ok(data.to_vec())
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserSaveStorageStateNode {}

impl BrowserSaveStorageStateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserSaveStorageStateNode {
    fn get_node(&self) -> Node {
        let mut node = super::manage::base_node(
            "browser_save_storage_state",
            "Save Storage State",
            "Saves all cookies (every domain, including HttpOnly) and the current origin's localStorage to a Playwright-compatible storageState JSON file. The file holds login sessions; store it like a password.",
        );
        node.category = "Automation/Browser/Auth".to_string();
        node.set_flowscript_name("browser", "saveStorageState");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(8)
                .set_governance(4)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "file_path",
            "File Path",
            "Where to write the storage state JSON",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_input_pin(
            "include_session_storage",
            "Include Session Storage",
            "Also save the current origin's sessionStorage",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_output_pin(
            "cookie_count",
            "Cookie Count",
            "Number of cookies saved",
            VariableType::Integer,
        );
        node.add_output_pin(
            "origin_count",
            "Origin Count",
            "Number of origins whose storage was saved",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let file_path: FlowPath = context.evaluate_pin("file_path").await?;
        let include_session: bool = context.evaluate_pin("include_session_storage").await?;
        let page = session.browser_page(context).await?;
        let cookies = read_cookies(&page).await?;
        let storage: OriginStorage =
            run_top_level(&page, READ_STORAGE, vec![json!(include_session)])
                .await
                .map_err(|error| {
                    flow_like_types::anyhow!("Failed to read page storage: {error}")
                })?;
        drop(page);
        let has_storage = !storage.local_storage.is_empty() || !storage.session_storage.is_empty();
        let state = StorageState {
            cookies,
            origins: if storage.origin != "null" && has_storage {
                vec![storage]
            } else {
                Vec::new()
            },
        };
        let bytes = flow_like_types::json::to_vec_pretty(&state)?;
        let runtime = file_path.to_runtime(context).await?;
        runtime
            .store
            .as_generic()
            .put(&runtime.path, bytes.into())
            .await
            .map_err(|error| {
                flow_like_types::anyhow!("Failed to write storage state {}: {error}", runtime.path)
            })?;
        context
            .set_pin_value("cookie_count", json!(state.cookies.len()))
            .await?;
        context
            .set_pin_value("origin_count", json!(state.origins.len()))
            .await?;
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
pub struct BrowserLoadStorageStateNode {}

impl BrowserLoadStorageStateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserLoadStorageStateNode {
    fn get_node(&self) -> Node {
        let mut node = super::manage::base_node(
            "browser_load_storage_state",
            "Load Storage State",
            "Restores cookies and web storage from a Playwright-compatible storageState JSON file. Cookies for every domain are restored. Storage is applied only to the origin the page is on, without navigating; other origins are reported as skipped.",
        );
        node.category = "Automation/Browser/Auth".to_string();
        node.set_flowscript_name("browser", "loadStorageState");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(8)
                .set_governance(4)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "file_path",
            "File Path",
            "Storage state JSON written by Save Storage State or Playwright",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_output_pin(
            "cookies_applied",
            "Cookies Applied",
            "Cookies the browser accepted",
            VariableType::Integer,
        );
        node.add_output_pin(
            "cookies_failed",
            "Cookies Failed",
            "Cookies that were expired or rejected",
            VariableType::Integer,
        );
        node.add_output_pin(
            "origins_applied",
            "Origins Applied",
            "Origins whose storage was restored",
            VariableType::Integer,
        );
        node.add_output_pin(
            "skipped_origins",
            "Skipped Origins",
            "Origins not restored because the page is on a different origin",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        let file_path: FlowPath = context.evaluate_pin("file_path").await?;
        let data = read_storage_file(context, &file_path).await?;
        let state: StorageState = flow_like_types::json::from_slice(&data).map_err(|error| {
            flow_like_types::anyhow!("Storage state file is not valid: {error}")
        })?;
        let page = session.browser_page(context).await?;
        let report = write_cookies(context, &page, &state.cookies).await?;
        let current_origin: String =
            run_top_level(&page, "return location.origin;", vec![]).await?;
        let mut origins_applied = 0;
        let mut skipped_origins = Vec::new();
        for origin in &state.origins {
            if origin.origin.trim_end_matches('/') != current_origin {
                skipped_origins.push(origin.origin.clone());
                continue;
            }
            run_top_level::<flow_like_types::Value>(
                &page,
                WRITE_STORAGE,
                vec![json!(origin.local_storage), json!(origin.session_storage)],
            )
            .await
            .map_err(|error| {
                flow_like_types::anyhow!("Failed to restore storage for {}: {error}", origin.origin)
            })?;
            origins_applied += 1;
        }
        drop(page);
        context
            .set_pin_value("cookies_applied", json!(report.applied))
            .await?;
        context
            .set_pin_value("cookies_failed", json!(report.failed))
            .await?;
        context
            .set_pin_value("origins_applied", json!(origins_applied))
            .await?;
        context
            .set_pin_value("skipped_origins", json!(skipped_origins))
            .await?;
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

    #[test]
    fn cdp_cookies_map_to_playwright_shape() {
        let cookie = StorageCookie::from_cdp(&json!({
            "name": "sid", "value": "abc", "domain": ".example.com", "path": "/",
            "expires": 1_900_000_000.5, "size": 6, "httpOnly": true, "secure": true,
            "session": false, "sameSite": "None", "priority": "Medium"
        }))
        .unwrap();
        assert_eq!(
            flow_like_types::json::to_value(&cookie).unwrap(),
            json!({
                "name": "sid", "value": "abc", "domain": ".example.com", "path": "/",
                "expires": 1_900_000_000.5, "httpOnly": true, "secure": true, "sameSite": "None"
            })
        );
        let session = StorageCookie::from_cdp(&json!({
            "name": "tmp", "value": "", "domain": "example.com", "path": "/app",
            "expires": -1, "httpOnly": false, "secure": false, "session": true
        }))
        .unwrap();
        assert_eq!(session.expires, -1.0);
        assert_eq!(session.same_site, "Lax");
        assert!(StorageCookie::from_cdp(&json!({ "value": "x" })).is_none());
    }

    #[test]
    fn cookie_params_keep_security_attributes() {
        let cookie = StorageCookie {
            name: "sid".into(),
            value: "abc".into(),
            domain: ".example.com".into(),
            path: "/".into(),
            expires: 1_900_000_000.0,
            http_only: true,
            secure: true,
            same_site: "strict".into(),
        };
        let param = cookie.to_cdp("https://example.com/");
        assert_eq!(param["httpOnly"], json!(true));
        assert_eq!(param["sameSite"], json!("Strict"));
        assert_eq!(param["domain"], json!(".example.com"));
        assert!(param.get("url").is_none());
        let hostless = StorageCookie {
            domain: String::new(),
            expires: -1.0,
            ..cookie
        };
        let param = hostless.to_cdp("https://app.example.com/login");
        assert_eq!(param["url"], json!("https://app.example.com/login"));
        assert!(param.get("expires").is_none());
        assert!(!hostless.expired_at(2_000_000_000.0));
        let old = StorageCookie {
            expires: 100.0,
            ..hostless
        };
        assert!(old.expired_at(200.0));
    }

    #[test]
    fn storage_state_round_trips_playwright_files() {
        let playwright = json!({
            "cookies": [{
                "name": "sid", "value": "abc", "domain": "example.com", "path": "/",
                "expires": -1.0, "httpOnly": true, "secure": false, "sameSite": "Lax"
            }],
            "origins": [{
                "origin": "https://example.com",
                "localStorage": [{ "name": "token", "value": "t" }]
            }]
        });
        let state: StorageState = flow_like_types::json::from_value(playwright.clone()).unwrap();
        assert_eq!(state.cookies[0].expires, -1.0);
        assert_eq!(state.origins[0].local_storage[0].name, "token");
        assert_eq!(flow_like_types::json::to_value(&state).unwrap(), playwright);
        let minimal: StorageState = flow_like_types::json::from_value(
            json!({ "cookies": [{ "name": "a", "value": "b" }] }),
        )
        .unwrap();
        assert_eq!(minimal.cookies[0].path, "/");
        assert_eq!(minimal.cookies[0].same_site, "Lax");
        assert!(minimal.origins.is_empty());
    }
}
