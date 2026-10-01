#[cfg(feature = "execute")]
use super::selector::optional_input;
use crate::types::handles::AutomationSession;
#[cfg(feature = "execute")]
use crate::types::handles::{BrowserContextOptions, BrowserType, ProxySettings};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(any(feature = "execute", test))]
use flow_like_browser::{
    attach::WebDriverProbe,
    launch::{BrowserKind, LaunchOptions, ProxyConfig, default_cache_dir},
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};
#[cfg(any(feature = "execute", test))]
use std::time::Duration;

#[cfg(any(feature = "execute", test))]
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);

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
            "Launches a local Chrome or Edge (or Chrome for Testing) with its own profile and connects over the DevTools protocol, optionally with a persistent profile, proxy, locale and relaxed certificate checks",
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
            "Legacy WebDriver URL. Loopback addresses are ignored (the browser is launched locally); remote WebDriver hosts are not supported",
            VariableType::String,
        )
        .set_default_value(Some(json!("http://localhost:9515")));

        node.add_input_pin(
            "browser_type",
            "Browser Type",
            "Chrome or Edge. Firefox and Safari are not supported yet",
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
            "Local directory for a persistent browser profile (cookies, storage, logins survive between runs). Chrome and Edge allow one browser per profile at a time.",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_optional(true).build());

        node.add_input_pin(
            "user_data_path",
            "Profile Path",
            "Absolute profile directory on this machine; used when Profile Directory is not connected",
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
            "DevTools endpoint of the launched browser (localhost:port)",
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
        let pins = OpenPins {
            browser_type: context.evaluate_pin("browser_type").await?,
            headless: context.evaluate_pin("headless").await?,
            viewport_width: context.evaluate_pin("viewport_width").await?,
            viewport_height: context.evaluate_pin("viewport_height").await?,
            user_agent: context.evaluate_pin("user_agent").await?,
            page_load_timeout: context.evaluate_pin("page_load_timeout").await?,
            user_data_dir: None,
            proxy_server: optional_input(context, "proxy_server", String::new()).await?,
            proxy_bypass: optional_input(context, "proxy_bypass", String::new()).await?,
            locale: optional_input(context, "locale", String::new()).await?,
            ignore_https_errors: optional_input(context, "ignore_https_errors", false).await?,
        };
        session.ensure_active(context).await?;
        if session.has_browser() {
            return Err(flow_like_types::anyhow!(
                "Close the attached browser before replacing it"
            ));
        }

        let mut launch = launch_options_from_pins(&pins)?;
        check_webdriver_url(&webdriver_url).await?;
        launch.user_data_dir = profile_directory(context)
            .await?
            .map(std::path::PathBuf::from);
        let options = session_options(&launch, webdriver_url);

        session.prepare_browser_slot(context).await?;
        let browser = flow_like_browser::Browser::launch(launch).await?;
        let debugger_address = browser.debugger_address().unwrap_or_default().to_owned();
        session
            .attach_cdp_browser(context, browser, &options)
            .await?;

        super::selector::optional_output(context, "debugger_address", json!(debugger_address))
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

#[cfg(any(feature = "execute", test))]
struct OpenPins {
    browser_type: String,
    headless: bool,
    viewport_width: i64,
    viewport_height: i64,
    user_agent: String,
    page_load_timeout: i64,
    user_data_dir: Option<String>,
    proxy_server: String,
    proxy_bypass: String,
    locale: String,
    ignore_https_errors: bool,
}

#[cfg(any(feature = "execute", test))]
fn launch_options_from_pins(pins: &OpenPins) -> flow_like_types::Result<LaunchOptions> {
    let kind = browser_kind(&pins.browser_type)?;
    let window_size = viewport(pins.viewport_width, pins.viewport_height)?;
    let page_load_timeout = u64::try_from(pins.page_load_timeout)
        .ok()
        .filter(|seconds| *seconds >= 1)
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Page load timeout must be at least 1 second (got {})",
                pins.page_load_timeout
            )
        })?;
    let proxy_server = pins.proxy_server.trim();
    let proxy = if proxy_server.is_empty() {
        None
    } else {
        flow_like_browser::launch::args::validate_proxy(proxy_server)?;
        Some(ProxyConfig {
            server: proxy_server.to_string(),
            bypass: bypass_hosts(&pins.proxy_bypass),
        })
    };
    Ok(LaunchOptions {
        kind,
        executable: None,
        headless: pins.headless,
        window_size,
        user_agent: Some(pins.user_agent.clone()).filter(|agent| !agent.is_empty()),
        user_data_dir: pins.user_data_dir.as_ref().map(std::path::PathBuf::from),
        proxy,
        locale: Some(pins.locale.trim().to_string()).filter(|locale| !locale.is_empty()),
        ignore_https_errors: pins.ignore_https_errors,
        cache_dir: default_cache_dir(),
        page_load_timeout: Duration::from_secs(page_load_timeout),
        launch_timeout: LAUNCH_TIMEOUT,
    })
}

#[cfg(any(feature = "execute", test))]
fn browser_kind(name: &str) -> flow_like_types::Result<BrowserKind> {
    match name {
        "Chrome" => Ok(BrowserKind::Chrome),
        "Edge" => Ok(BrowserKind::Edge),
        "Firefox" | "Safari" => Err(flow_like_types::anyhow!(
            "Firefox and Safari are not supported by the new browser engine yet (WebDriver BiDi support is planned); use Chrome or Edge"
        )),
        other => Err(flow_like_types::anyhow!(
            "Unknown browser type '{other}' (use Chrome, Firefox, Edge or Safari)"
        )),
    }
}

#[cfg(any(feature = "execute", test))]
fn viewport(width: i64, height: i64) -> flow_like_types::Result<(u32, u32)> {
    match (u32::try_from(width), u32::try_from(height)) {
        (Ok(columns), Ok(rows)) if columns > 0 && rows > 0 => Ok((columns, rows)),
        _ => Err(flow_like_types::anyhow!(
            "Viewport must be positive (got {width}x{height})"
        )),
    }
}

#[cfg(feature = "execute")]
fn session_options(launch: &LaunchOptions, webdriver_url: String) -> BrowserContextOptions {
    BrowserContextOptions {
        browser_type: match launch.kind {
            BrowserKind::Chrome => BrowserType::Chrome,
            BrowserKind::Edge => BrowserType::Edge,
        },
        headless: launch.headless,
        user_data_dir: launch
            .user_data_dir
            .as_ref()
            .map(|directory| directory.to_string_lossy().into_owned()),
        viewport_width: Some(launch.window_size.0),
        viewport_height: Some(launch.window_size.1),
        user_agent: launch.user_agent.clone(),
        locale: launch.locale.clone(),
        ignore_https_errors: launch.ignore_https_errors,
        proxy: launch.proxy.as_ref().map(|proxy| ProxySettings {
            server: proxy.server.clone(),
            bypass: Some(proxy.bypass.join(",")).filter(|bypass| !bypass.is_empty()),
            username: None,
            password: None,
        }),
        webdriver_url: Some(webdriver_url),
        ..Default::default()
    }
}

#[cfg(any(feature = "execute", test))]
fn route_webdriver_url(url: &str, probe: Option<WebDriverProbe>) -> flow_like_types::Result<()> {
    if !flow_like_browser::attach::is_loopback(url) {
        return Err(flow_like_types::anyhow!(
            "Remote WebDriver hosts are no longer supported; use Attach to Browser with a CDP endpoint"
        ));
    }
    match probe {
        Some(WebDriverProbe::Other { name }) => {
            let name = webdriver_server_name(&name);
            Err(flow_like_types::anyhow!(
                "{url} is a {name} server; remote WebDriver hosts are no longer supported. Use Attach to Browser with a CDP endpoint"
            ))
        }
        _ => Ok(()),
    }
}

/// The probe names a server by its `/status` message, which for Selenium Grid is a readiness
/// sentence such as "Selenium Grid ready." or "Selenium Grid not ready.".
#[cfg(any(feature = "execute", test))]
fn webdriver_server_name(status_message: &str) -> &str {
    [" not ready.", " ready."]
        .into_iter()
        .find_map(|readiness| status_message.strip_suffix(readiness))
        .filter(|name| !name.is_empty())
        .unwrap_or(status_message)
}

#[cfg(feature = "execute")]
pub(crate) async fn check_webdriver_url(url: &str) -> flow_like_types::Result<()> {
    let url = url.trim();
    let probe = if flow_like_browser::attach::is_loopback(url) {
        Some(flow_like_browser::attach::probe_webdriver(url).await)
    } else {
        None
    };
    route_webdriver_url(url, probe)
}

#[cfg(feature = "execute")]
async fn profile_directory(
    context: &mut ExecutionContext,
) -> flow_like_types::Result<Option<String>> {
    let value = optional_input(context, "user_data_dir", flow_like_types::Value::Null).await?;
    if !value.is_null() && !value.as_object().is_some_and(|object| object.is_empty()) {
        let path: FlowPath = flow_like_types::json::from_value(value)?;
        let runtime = path.to_runtime(context).await?;
        let directory = match runtime.store.as_ref() {
            flow_like_storage::files::store::FlowLikeStore::Local(store) => {
                store.path_to_filesystem(&runtime.path)?
            }
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Profile Directory must be a local directory on this machine"
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
    let native: String = optional_input(context, "user_data_path", String::new()).await?;
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
fn bypass_hosts(bypass: &str) -> Vec<String> {
    bypass
        .split([',', ';'])
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_string)
        .collect()
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

    const REMOTE_WEBDRIVER: &str =
        "Remote WebDriver hosts are no longer supported; use Attach to Browser with a CDP endpoint";

    fn default_pins() -> OpenPins {
        OpenPins {
            browser_type: "Chrome".to_string(),
            headless: true,
            viewport_width: 1920,
            viewport_height: 1080,
            user_agent: String::new(),
            page_load_timeout: 30,
            user_data_dir: None,
            proxy_server: String::new(),
            proxy_bypass: String::new(),
            locale: String::new(),
            ignore_https_errors: false,
        }
    }

    fn open_error(pins: OpenPins) -> String {
        launch_options_from_pins(&pins)
            .expect_err("the pins must be rejected")
            .to_string()
    }

    #[test]
    fn proxy_bypass_hosts_split_on_commas_and_semicolons() {
        assert_eq!(
            bypass_hosts("localhost, *.internal;;10.0.0.1"),
            vec!["localhost", "*.internal", "10.0.0.1"]
        );
    }

    #[test]
    fn default_pins_launch_headless_chrome_with_a_temporary_profile() {
        let options = launch_options_from_pins(&default_pins()).expect("default pins launch");
        assert_eq!(options.kind, BrowserKind::Chrome);
        assert!(options.executable.is_none());
        assert!(options.headless);
        assert_eq!(options.window_size, (1920, 1080));
        assert_eq!(options.user_agent, None);
        assert_eq!(options.user_data_dir, None);
        assert!(options.proxy.is_none());
        assert_eq!(options.locale, None);
        assert!(!options.ignore_https_errors);
        assert_eq!(options.cache_dir, default_cache_dir());
        assert_eq!(options.page_load_timeout, Duration::from_secs(30));
        assert_eq!(options.launch_timeout, Duration::from_secs(30));
    }

    #[test]
    fn every_open_pin_maps_to_its_launch_option() {
        let options = launch_options_from_pins(&OpenPins {
            browser_type: "Edge".to_string(),
            headless: false,
            viewport_width: 800,
            viewport_height: 600,
            user_agent: "FlowLike/1.0".to_string(),
            page_load_timeout: 5,
            user_data_dir: Some("/profiles/crm".to_string()),
            proxy_server: " socks5://127.0.0.1:1080 ".to_string(),
            proxy_bypass: "localhost, *.internal".to_string(),
            locale: " de-DE ".to_string(),
            ignore_https_errors: true,
        })
        .expect("valid pins launch");
        assert_eq!(options.kind, BrowserKind::Edge);
        assert!(!options.headless);
        assert_eq!(options.window_size, (800, 600));
        assert_eq!(options.user_agent.as_deref(), Some("FlowLike/1.0"));
        assert_eq!(
            options.user_data_dir,
            Some(std::path::PathBuf::from("/profiles/crm"))
        );
        let proxy = options.proxy.expect("proxy is configured");
        assert_eq!(proxy.server, "socks5://127.0.0.1:1080");
        assert_eq!(proxy.bypass, vec!["localhost", "*.internal"]);
        assert_eq!(options.locale.as_deref(), Some("de-DE"));
        assert!(options.ignore_https_errors);
        assert_eq!(options.page_load_timeout, Duration::from_secs(5));
    }

    #[test]
    fn proxy_bypass_without_a_proxy_server_is_ignored() {
        let options = launch_options_from_pins(&OpenPins {
            proxy_server: "  ".to_string(),
            proxy_bypass: "localhost".to_string(),
            ..default_pins()
        })
        .expect("pins launch");
        assert!(options.proxy.is_none());
    }

    #[test]
    fn proxy_credentials_are_rejected_without_echoing_them() {
        let message = open_error(OpenPins {
            proxy_server: "http://user:secret@proxy.local:3128".to_string(),
            ..default_pins()
        });
        assert!(
            message.contains("Proxy credentials are not supported"),
            "{message}"
        );
        assert!(!message.contains("secret"), "{message}");
    }

    #[test]
    fn firefox_and_safari_are_rejected_with_the_engine_error() {
        for browser_type in ["Firefox", "Safari"] {
            assert_eq!(
                open_error(OpenPins {
                    browser_type: browser_type.to_string(),
                    ..default_pins()
                }),
                "Firefox and Safari are not supported by the new browser engine yet (WebDriver BiDi support is planned); use Chrome or Edge"
            );
        }
        assert_eq!(
            open_error(OpenPins {
                browser_type: "Opera".to_string(),
                ..default_pins()
            }),
            "Unknown browser type 'Opera' (use Chrome, Firefox, Edge or Safari)"
        );
    }

    #[test]
    fn viewport_and_page_load_timeout_keep_their_limits() {
        for (width, height) in [(0, 1080), (1920, -1), (i64::from(u32::MAX) + 1, 1080)] {
            assert_eq!(
                open_error(OpenPins {
                    viewport_width: width,
                    viewport_height: height,
                    ..default_pins()
                }),
                format!("Viewport must be positive (got {width}x{height})")
            );
        }
        for timeout in [0, -5] {
            assert_eq!(
                open_error(OpenPins {
                    page_load_timeout: timeout,
                    ..default_pins()
                }),
                format!("Page load timeout must be at least 1 second (got {timeout})")
            );
        }
    }

    #[test]
    fn remote_webdriver_hosts_are_rejected() {
        for url in [
            "http://grid.example.com:4444",
            "http://10.0.0.5:9515",
            "selenium:4444",
        ] {
            let error = route_webdriver_url(url, None).expect_err("remote host");
            assert_eq!(error.to_string(), REMOTE_WEBDRIVER, "{url}");
        }
        let error = route_webdriver_url(
            "http://grid.example.com:4444",
            Some(WebDriverProbe::Chromedriver),
        )
        .expect_err("remote host even when a probe answered");
        assert_eq!(error.to_string(), REMOTE_WEBDRIVER);
    }

    #[test]
    fn loopback_webdriver_urls_launch_locally_unless_another_webdriver_answers() {
        for url in [
            "",
            "http://localhost:9515",
            "http://127.0.0.1:9515",
            "http://[::1]:9515",
            "localhost:9515",
        ] {
            for probe in [
                None,
                Some(WebDriverProbe::NotRunning),
                Some(WebDriverProbe::Chromedriver),
            ] {
                assert!(
                    route_webdriver_url(url, probe.clone()).is_ok(),
                    "{url} {probe:?}"
                );
            }
        }
        let error = route_webdriver_url(
            "http://127.0.0.1:4444",
            Some(WebDriverProbe::Other {
                name: "geckodriver".to_string(),
            }),
        )
        .expect_err("another WebDriver server");
        assert_eq!(
            error.to_string(),
            "http://127.0.0.1:4444 is a geckodriver server; remote WebDriver hosts are no longer supported. Use Attach to Browser with a CDP endpoint"
        );
    }

    #[test]
    fn selenium_grid_is_named_without_its_readiness_sentence() {
        for status_message in ["Selenium Grid ready.", "Selenium Grid not ready."] {
            let error = route_webdriver_url(
                "http://localhost:4444/wd/hub",
                Some(WebDriverProbe::Other {
                    name: status_message.to_string(),
                }),
            )
            .expect_err("Selenium Grid");
            assert_eq!(
                error.to_string(),
                "http://localhost:4444/wd/hub is a Selenium Grid server; remote WebDriver hosts are no longer supported. Use Attach to Browser with a CDP endpoint",
                "{status_message}"
            );
        }
    }
}
