use crate::types::handles::AutomationSession;
#[cfg(feature = "execute")]
use crate::types::handles::{BrowserContextOptions, BrowserType, ProxySettings};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use std::time::Duration;
#[cfg(feature = "execute")]
use thirtyfour::{
    Capabilities, CapabilitiesHelper, DesiredCapabilities,
    common::capabilities::chromium::ChromiumLikeCapabilities,
};

#[crate::register_node]
#[derive(Default)]
pub struct BrowserOpenNode {}

impl BrowserOpenNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserOpenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_open",
            "Open Browser",
            "Connects to a WebDriver server and opens a new browser session, optionally with a persistent profile, proxy, locale and relaxed certificate checks",
            "Automation/Browser",
        );
        node.set_version(2);
        node.set_flowscript_name("browser", "open");
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session to attach browser to",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "webdriver_url",
            "WebDriver URL",
            "URL of the WebDriver server (e.g., http://localhost:9515 for ChromeDriver)",
            VariableType::String,
        )
        .set_default_value(Some(json!("http://localhost:9515")));

        node.add_input_pin(
            "browser_type",
            "Browser Type",
            "Browser to use (Chrome, Firefox, Edge, Safari)",
            VariableType::String,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(vec![
                    "Chrome".to_string(),
                    "Firefox".to_string(),
                    "Edge".to_string(),
                    "Safari".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Chrome")));

        node.add_input_pin(
            "headless",
            "Headless",
            "Run browser in headless mode (no visible window)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "viewport_width",
            "Viewport Width",
            "Page viewport width in CSS pixels; the window grows by the browser frame so the page gets this size (the screen may cap it)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1920)));

        node.add_input_pin(
            "viewport_height",
            "Viewport Height",
            "Page viewport height in CSS pixels; the window grows by the browser frame so the page gets this size (the screen may cap it)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1080)));

        node.add_input_pin(
            "user_agent",
            "User Agent",
            "Custom user agent string (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "page_load_timeout",
            "Page Load Timeout (s)",
            "Timeout for page loads in seconds (at least 1)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));

        node.add_input_pin(
            "user_data_dir",
            "Profile Directory",
            "Local directory for a persistent browser profile (cookies, storage, logins survive between runs). Chrome and Edge allow one browser per profile at a time. Requires WebDriver on this machine.",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_optional(true).build());

        node.add_input_pin(
            "user_data_path",
            "Profile Path",
            "Absolute profile directory on the WebDriver host; used when Profile Directory is not connected",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "proxy_server",
            "Proxy Server",
            "Proxy such as http://host:8080 or socks5://host:1080 (proxy credentials are not supported)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "proxy_bypass",
            "Proxy Bypass",
            "Comma-separated hosts that skip the proxy, such as localhost,*.internal",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "locale",
            "Locale",
            "Browser language and Accept-Language such as de-DE (empty keeps the default)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "ignore_https_errors",
            "Ignore HTTPS Errors",
            "Accept invalid or self-signed TLS certificates",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "debugger_address",
            "Debugger Address",
            "Chrome or Edge debugger endpoint, when available",
            VariableType::String,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session with browser attached",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        let webdriver_url: String = context.evaluate_pin("webdriver_url").await?;
        let browser_type_str: String = context.evaluate_pin("browser_type").await?;
        let headless: bool = context.evaluate_pin("headless").await?;
        let viewport_width: i64 = context.evaluate_pin("viewport_width").await?;
        let viewport_height: i64 = context.evaluate_pin("viewport_height").await?;
        let user_agent: String = context.evaluate_pin("user_agent").await?;
        let page_load_timeout: i64 = context.evaluate_pin("page_load_timeout").await?;
        let proxy_server: String =
            super::selector::optional_input(context, "proxy_server", String::new()).await?;
        let proxy_bypass: String =
            super::selector::optional_input(context, "proxy_bypass", String::new()).await?;
        let locale: String =
            super::selector::optional_input(context, "locale", String::new()).await?;
        let ignore_https_errors: bool =
            super::selector::optional_input(context, "ignore_https_errors", false).await?;
        session.ensure_active(context).await?;
        if session.has_browser() {
            return Err(flow_like_types::anyhow!(
                "Close the attached browser before replacing it"
            ));
        }

        let browser_type = match browser_type_str.as_str() {
            "Firefox" => BrowserType::Firefox,
            "Edge" => BrowserType::Edge,
            "Safari" => BrowserType::Safari,
            "Chrome" => BrowserType::Chrome,
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unknown browser type '{other}' (use Chrome, Firefox, Edge or Safari)"
                ));
            }
        };
        let (Ok(width), Ok(height)) = (
            u32::try_from(viewport_width),
            u32::try_from(viewport_height),
        ) else {
            return Err(flow_like_types::anyhow!(
                "Viewport must be positive (got {viewport_width}x{viewport_height})"
            ));
        };
        if width == 0 || height == 0 {
            return Err(flow_like_types::anyhow!(
                "Viewport must be positive (got {viewport_width}x{viewport_height})"
            ));
        }
        if page_load_timeout < 1 {
            return Err(flow_like_types::anyhow!(
                "Page load timeout must be at least 1 second (got {page_load_timeout})"
            ));
        }

        let options = BrowserContextOptions {
            browser_type,
            headless,
            user_data_dir: profile_directory(context).await?,
            viewport_width: Some(width),
            viewport_height: Some(height),
            user_agent: Some(user_agent).filter(|agent| !agent.is_empty()),
            locale: Some(locale.trim().to_string()).filter(|locale| !locale.is_empty()),
            ignore_https_errors,
            proxy: Some(proxy_server.trim().to_string())
                .filter(|server| !server.is_empty())
                .map(|server| ProxySettings {
                    server,
                    bypass: Some(proxy_bypass.trim().to_string())
                        .filter(|bypass| !bypass.is_empty()),
                    username: None,
                    password: None,
                }),
            webdriver_url: Some(webdriver_url.clone()),
            ..Default::default()
        };
        let caps = browser_capabilities(&options)?;

        let (driver, debugger_address) = super::protocol::connect_webdriver(&webdriver_url, caps)
            .await
            .map_err(|e| {
                flow_like_types::anyhow!(
                    "Failed to connect to WebDriver at {}: {}",
                    webdriver_url,
                    e
                )
            })?;

        let setup: flow_like_types::Result<thirtyfour::WindowHandle> = async {
            driver
                .set_page_load_timeout(Duration::from_secs(page_load_timeout as u64))
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to set page load timeout: {}", e))?;
            match_viewport(&driver, width, height).await?;
            Ok(driver.window().await?)
        }
        .await;
        let initial_window = match setup {
            Ok(handle) => handle,
            Err(error) => {
                let _ = driver.quit().await;
                return Err(error);
            }
        };

        if let Err(error) = session
            .attach_browser(context, driver.clone(), &options)
            .await
        {
            let _ = driver.quit().await;
            return Err(error);
        }
        session.set_current_page(context, initial_window).await?;
        if let Some(address) = &debugger_address {
            super::protocol::remember_debugger_address(context, &session, address).await;
        }
        super::selector::optional_output(
            context,
            "debugger_address",
            json!(debugger_address.unwrap_or_default()),
        )
        .await?;

        super::selector::optional_output(context, "session_out", json!(session)).await?;
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

#[cfg(feature = "execute")]
async fn profile_directory(
    context: &mut ExecutionContext,
) -> flow_like_types::Result<Option<String>> {
    let value = super::selector::optional_input(
        context,
        "user_data_dir",
        flow_like_types::Value::Null,
    )
    .await?;
    if !value.is_null() && !value.as_object().is_some_and(|object| object.is_empty()) {
        let path: FlowPath = flow_like_types::json::from_value(value)?;
        let runtime = path.to_runtime(context).await?;
        let directory = match runtime.store.as_ref() {
            flow_like_storage::files::store::FlowLikeStore::Local(store) => {
                store.path_to_filesystem(&runtime.path)?
            }
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Profile Directory must be a local directory on the WebDriver host"
                ));
            }
        };
        std::fs::create_dir_all(&directory).map_err(|error| {
            flow_like_types::anyhow!(
                "Failed to create profile directory {}: {error}",
                directory.display()
            )
        })?;
        return Ok(Some(directory.to_string_lossy().into_owned()));
    }
    let native: String =
        super::selector::optional_input(context, "user_data_path", String::new()).await?;
    let native = native.trim();
    if native.is_empty() {
        return Ok(None);
    }
    if !std::path::Path::new(native).is_absolute() {
        return Err(flow_like_types::anyhow!(
            "Profile Path must be absolute (got '{native}')"
        ));
    }
    Ok(Some(native.to_string()))
}

#[cfg(any(feature = "execute", test))]
fn proxy_parts(server: &str) -> (String, String) {
    match server.split_once("://") {
        Some((scheme, rest)) => (
            scheme.to_ascii_lowercase(),
            rest.trim_end_matches('/').to_string(),
        ),
        None => ("http".to_string(), server.trim_end_matches('/').to_string()),
    }
}

#[cfg(any(feature = "execute", test))]
fn bypass_hosts(bypass: &str) -> Vec<String> {
    bypass
        .split([',', ';'])
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(feature = "execute")]
fn w3c_proxy(proxy: &ProxySettings) -> flow_like_types::Result<thirtyfour::common::capabilities::desiredcapabilities::Proxy> {
    use thirtyfour::common::capabilities::desiredcapabilities::Proxy;
    let (scheme, address) = proxy_parts(&proxy.server);
    let no_proxy = proxy.bypass.as_deref().map(bypass_hosts);
    let (http, socks, socks_version) = match scheme.as_str() {
        "http" | "https" => (Some(address), None, None),
        "socks5" | "socks5h" => (None, Some(address), Some(5)),
        "socks4" | "socks4a" => (None, Some(address), Some(4)),
        other => {
            return Err(flow_like_types::anyhow!(
                "Unsupported proxy scheme '{other}' in '{}' (use http, https, socks4 or socks5)",
                proxy.server
            ));
        }
    };
    Ok(Proxy::Manual {
        ftp_proxy: None,
        ssl_proxy: http.clone(),
        http_proxy: http,
        socks_proxy: socks,
        socks_version,
        socks_username: None,
        socks_password: None,
        no_proxy,
    })
}

#[cfg(feature = "execute")]
fn chromium_options(
    caps: &mut impl ChromiumLikeCapabilities,
    options: &BrowserContextOptions,
) -> flow_like_types::Result<()> {
    let failed = |setting: &str, error: thirtyfour::error::WebDriverError| {
        flow_like_types::anyhow!("Failed to set {setting}: {error}")
    };
    if options.headless {
        caps.set_headless().map_err(|e| failed("headless mode", e))?;
    }
    if let (Some(width), Some(height)) = (options.viewport_width, options.viewport_height) {
        caps.add_arg(&format!("--window-size={width},{height}"))
            .map_err(|e| failed("window size", e))?;
    }
    if let Some(agent) = &options.user_agent {
        caps.add_arg(&format!("--user-agent={agent}"))
            .map_err(|e| failed("user agent", e))?;
    }
    if let Some(directory) = &options.user_data_dir {
        caps.add_arg(&format!("--user-data-dir={directory}"))
            .map_err(|e| failed("profile directory", e))?;
    }
    if let Some(proxy) = &options.proxy {
        caps.add_arg(&format!("--proxy-server={}", proxy.server))
            .map_err(|e| failed("proxy", e))?;
        if let Some(bypass) = &proxy.bypass {
            caps.add_arg(&format!("--proxy-bypass-list={}", bypass_hosts(bypass).join(";")))
                .map_err(|e| failed("proxy bypass", e))?;
        }
    }
    if let Some(locale) = &options.locale {
        caps.add_arg(&format!("--lang={locale}"))
            .map_err(|e| failed("locale", e))?;
        caps.add_experimental_option("prefs", json!({ "intl.accept_languages": locale }))
            .map_err(|e| failed("locale", e))?;
    }
    if options.ignore_https_errors {
        caps.accept_insecure_certs(true)
            .map_err(|e| failed("certificate handling", e))?;
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn browser_capabilities(options: &BrowserContextOptions) -> flow_like_types::Result<Capabilities> {
    match options.browser_type {
        BrowserType::Chrome => {
            let mut caps = DesiredCapabilities::chrome();
            chromium_options(&mut caps, options)?;
            Ok(Capabilities::from(caps))
        }
        BrowserType::Edge => {
            let mut caps = DesiredCapabilities::edge();
            chromium_options(&mut caps, options)?;
            Ok(Capabilities::from(caps))
        }
        BrowserType::Firefox => {
            let mut caps = DesiredCapabilities::firefox();
            if options.headless {
                caps.set_headless()
                    .map_err(|e| flow_like_types::anyhow!("Failed to set headless: {}", e))?;
            }
            let mut prefs = thirtyfour::common::capabilities::firefox::FirefoxPreferences::new();
            if let Some(agent) = &options.user_agent {
                prefs.set("general.useragent.override", agent)?;
            }
            if let Some(locale) = &options.locale {
                prefs.set("intl.accept_languages", locale)?;
                prefs.set("intl.locale.requested", locale)?;
            }
            caps.set_preferences(prefs)?;
            if let Some(directory) = &options.user_data_dir {
                caps.add_arg("-profile")?;
                caps.add_arg(directory)?;
            }
            if let Some(proxy) = &options.proxy {
                caps.set_proxy(w3c_proxy(proxy)?)?;
            }
            if options.ignore_https_errors {
                caps.accept_insecure_certs(true)?;
            }
            Ok(Capabilities::from(caps))
        }
        BrowserType::Safari => {
            if options.headless || options.user_agent.is_some() {
                return Err(flow_like_types::anyhow!(
                    "Safari WebDriver does not support headless mode or a custom user agent"
                ));
            }
            if options.user_data_dir.is_some()
                || options.proxy.is_some()
                || options.locale.is_some()
                || options.ignore_https_errors
            {
                return Err(flow_like_types::anyhow!(
                    "Safari WebDriver does not support a profile directory, proxy, locale or ignoring HTTPS errors"
                ));
            }
            Ok(Capabilities::from(DesiredCapabilities::safari()))
        }
    }
}

/// Sizes the window so the page viewport (not the outer window) matches the request.
#[cfg(feature = "execute")]
async fn match_viewport(
    driver: &thirtyfour::WebDriver,
    width: u32,
    height: u32,
) -> flow_like_types::Result<()> {
    driver
        .set_window_rect(0, 0, width, height)
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to set window size: {}", e))?;
    let Ok(inner) = driver
        .execute("return [window.innerWidth, window.innerHeight];", vec![])
        .await
        .and_then(|result| result.convert::<[i64; 2]>())
    else {
        return Ok(());
    };
    let extra_width = (i64::from(width) - inner[0]).max(0);
    let extra_height = (i64::from(height) - inner[1]).max(0);
    if extra_width == 0 && extra_height == 0 {
        return Ok(());
    }
    let grown = |size: u32, extra: i64| u32::try_from(i64::from(size) + extra).unwrap_or(size);
    driver
        .set_window_rect(0, 0, grown(width, extra_width), grown(height, extra_height))
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to fit the viewport to {width}x{height}: {e}"))?;
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserCloseNode {}

impl BrowserCloseNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserCloseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_close",
            "Close Browser",
            "Closes a browser started by this session, or disconnects from an existing browser",
            "Automation/Browser",
        );
        node.set_version(1);
        node.set_flowscript_name("browser", "close");
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session with browser to close",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Session with browser detached",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut session: AutomationSession = context.evaluate_pin("session").await?;
        let detached = session.detach_browser(context).await;
        super::protocol::clear_listeners(context, &session).await;
        {
            let mut cache = context.cache.write().await;
            cache.remove(&format!("automation:debugger:{}", session.session_ref));
            cache.remove(&super::refs::refs_key(&session));
        }
        detached?;
        super::selector::optional_output(context, "session_out", json!(session)).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_servers_split_into_scheme_and_address() {
        assert_eq!(
            proxy_parts("socks5://127.0.0.1:1080/"),
            ("socks5".to_string(), "127.0.0.1:1080".to_string())
        );
        assert_eq!(
            proxy_parts("proxy.local:3128"),
            ("http".to_string(), "proxy.local:3128".to_string())
        );
        assert_eq!(
            bypass_hosts("localhost, *.internal;;10.0.0.1"),
            vec!["localhost", "*.internal", "10.0.0.1"]
        );
    }
}
