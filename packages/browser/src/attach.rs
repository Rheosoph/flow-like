// Derived from agent-browser cli/src/native/cdp/chrome.rs @d01253d, Copyright 2025 Vercel Inc., Apache-2.0; modified by Rheosoph GmbH. See NOTICE.
use std::error::Error;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use url::{Host, Url};

use crate::error::BrowserError;
use crate::launch::{BrowserKind, Flavor};
use crate::transport::tls::{HttpProxy, http_client};
use crate::transport::ws::redact_url;

const APPROVAL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(120);
const DIRECT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const ACTIVE_PORT_FILE: &str = "DevToolsActivePort";
const BROWSER_WS_PATH: &str = "/devtools/browser";
const ERROR_BODY_CHARS: usize = 200;
const CHROMEDRIVER_MARKERS: [&str; 3] =
    ["ChromeDriver", "msedgedriver", "Microsoft Edge WebDriver"];

#[derive(Clone, Debug, PartialEq)]
pub enum AttachEndpoint {
    PortMode {
        ws_url: String,
    },
    ApprovalMode {
        ws_url: String,
        handshake_timeout: std::time::Duration,
    },
    DirectWs {
        ws_url: String,
        handshake_timeout: std::time::Duration,
    },
}

impl AttachEndpoint {
    pub fn needs_approval(&self) -> bool {
        matches!(self, Self::ApprovalMode { .. })
    }

    pub fn ws_url(&self) -> &str {
        match self {
            Self::PortMode { ws_url }
            | Self::ApprovalMode { ws_url, .. }
            | Self::DirectWs { ws_url, .. } => ws_url,
        }
    }
}

pub async fn resolve_endpoint(
    address: &str,
    kind: crate::launch::BrowserKind,
) -> crate::Result<AttachEndpoint> {
    resolve_endpoint_with(address, kind, default_profile_dir(kind).as_deref()).await
}

pub async fn resolve_endpoint_with(
    address: &str,
    kind: crate::launch::BrowserKind,
    default_profile: Option<&std::path::Path>,
) -> crate::Result<AttachEndpoint> {
    let address = address.trim();
    if address.is_empty() {
        return approval_endpoint_from_profile(kind, default_profile);
    }
    if has_websocket_scheme(address) {
        return websocket_endpoint(address);
    }
    discover_port_endpoint(address, kind).await
}

pub fn read_devtools_active_port(dir: &std::path::Path) -> crate::Result<Option<(u16, String)>> {
    let path = dir.join(ACTIVE_PORT_FILE);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(BrowserError::io(
                format!("Could not read {}", path.display()),
                &error,
            ));
        }
    };
    if content.trim().is_empty() {
        return Ok(None);
    }
    parse_active_port(&content).map(Some).map_err(|problem| {
        connect_error(format!(
            "{} is not a valid DevToolsActivePort file ({problem}); expected a port line and a {BROWSER_WS_PATH} path line",
            path.display()
        ))
    })
}

pub fn default_profile_dir(kind: crate::launch::BrowserKind) -> Option<std::path::PathBuf> {
    let flavor = match kind {
        BrowserKind::Chrome => Flavor::Chrome,
        BrowserKind::Edge => Flavor::Edge,
    };
    crate::launch::profile::default_user_data_dir(flavor)
}

#[derive(Clone, Debug, PartialEq)]
pub enum WebDriverProbe {
    NotRunning,
    Chromedriver,
    Other { name: String },
}

pub fn is_loopback(url: &str) -> bool {
    let url = url.trim();
    url.is_empty()
        || Url::parse(&with_http_scheme(url)).is_ok_and(|parsed| is_loopback_host(parsed.host()))
}

pub async fn probe_webdriver(url: &str) -> WebDriverProbe {
    let Some(status_url) = webdriver_status_url(url) else {
        return WebDriverProbe::NotRunning;
    };
    let client = match http_client(&[], PROBE_TIMEOUT, HttpProxy::None) {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!("WebDriver probe skipped, treating it as not running: {error}");
            return WebDriverProbe::NotRunning;
        }
    };
    let body = match client.get(status_url.clone()).send().await {
        Ok(response) => response.bytes().await,
        Err(error) => Err(error),
    };
    match body {
        Ok(body) => classify_webdriver_status(&body),
        Err(error) => {
            tracing::debug!(
                "WebDriver probe of {} found nothing: {}",
                redact_url(status_url.as_str()),
                error_chain(error)
            );
            WebDriverProbe::NotRunning
        }
    }
}

fn approval_endpoint_from_profile(
    kind: BrowserKind,
    profile: Option<&Path>,
) -> crate::Result<AttachEndpoint> {
    let active_port = match profile {
        Some(dir) => read_devtools_active_port(dir)?,
        None => None,
    };
    let Some((port, path)) = active_port else {
        let browser = browser_name(kind);
        return Err(connect_error(format!(
            "Remote debugging is not enabled in {browser}. Open chrome://inspect/#remote-debugging and turn on 'Allow remote debugging for this browser instance', then run again."
        )));
    };
    Ok(AttachEndpoint::ApprovalMode {
        ws_url: format!("ws://127.0.0.1:{port}{path}"),
        handshake_timeout: APPROVAL_HANDSHAKE_TIMEOUT,
    })
}

fn websocket_endpoint(address: &str) -> crate::Result<AttachEndpoint> {
    let url = Url::parse(address).map_err(|error| {
        connect_error(format!(
            "Debugger Address is not a valid WebSocket URL ({error})"
        ))
    })?;
    let ws_url = address.to_owned();
    if is_loopback_host(url.host()) && url.path().starts_with(BROWSER_WS_PATH) {
        return Ok(AttachEndpoint::ApprovalMode {
            ws_url,
            handshake_timeout: APPROVAL_HANDSHAKE_TIMEOUT,
        });
    }
    Ok(AttachEndpoint::DirectWs {
        ws_url,
        handshake_timeout: DIRECT_HANDSHAKE_TIMEOUT,
    })
}

async fn discover_port_endpoint(address: &str, kind: BrowserKind) -> crate::Result<AttachEndpoint> {
    let base = discovery_base(address)?;
    let shown = shown_address(address, &base);
    let body = fetch_json_version(&base, &shown, kind).await?;
    let reported = web_socket_debugger_url(&shown, &body)?;
    Ok(AttachEndpoint::PortMode {
        ws_url: rewrite_to_requested_host(&reported, &base, &shown)?,
    })
}

async fn fetch_json_version(base: &Url, shown: &str, kind: BrowserKind) -> crate::Result<String> {
    let version_url = base.join("/json/version").map_err(|error| {
        connect_error(format!(
            "Could not build the /json/version URL for {shown} ({error})"
        ))
    })?;
    let client = http_client(&[], DISCOVERY_TIMEOUT, HttpProxy::None)?;
    let response = client
        .get(version_url)
        .send()
        .await
        .map_err(|error| discovery_send_error(shown, kind, error))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        let browser = browser_name(kind);
        return Err(connect_error(format!(
            "{shown} is the remote-debugging consent endpoint of your everyday {browser} profile, not a dedicated debugging browser. To let this flow control your everyday {browser}, clear Debugger Address."
        )));
    }
    let body = response.text().await.map_err(|error| {
        connect_error(format!(
            "Could not read the /json/version reply of {shown}: {}",
            error_chain(error)
        ))
    })?;
    if status != reqwest::StatusCode::OK {
        let detail: String = body.trim().chars().take(ERROR_BODY_CHARS).collect();
        return Err(connect_error(format!(
            "GET /json/version on {shown} answered HTTP {status} ({detail}); expected 200 from a browser started with --remote-debugging-port"
        )));
    }
    Ok(body)
}

fn discovery_base(address: &str) -> crate::Result<Url> {
    let base = Url::parse(&with_http_scheme(address)).map_err(|error| {
        connect_error(format!(
            "Debugger Address is not host:port, an http(s):// URL or a ws(s):// URL ({error})"
        ))
    })?;
    if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() {
        return Err(connect_error(format!(
            "Debugger Address {} is not host:port, an http(s):// URL or a ws(s):// URL",
            shown_address(address, &base)
        )));
    }
    Ok(base)
}

fn discovery_send_error(shown: &str, kind: BrowserKind, error: reqwest::Error) -> BrowserError {
    if is_connection_refused(&error) {
        let browser = browser_name(kind);
        return connect_error(format!(
            "Nothing is listening on {shown}. Start {browser} with --remote-debugging-port and its own --user-data-dir, or clear Debugger Address to use your everyday {browser} after enabling chrome://inspect/#remote-debugging."
        ));
    }
    if error.is_timeout() {
        return connect_error(format!(
            "{shown} did not answer GET /json/version within {} s",
            DISCOVERY_TIMEOUT.as_secs()
        ));
    }
    connect_error(format!(
        "GET /json/version on {shown} failed: {}",
        error_chain(error)
    ))
}

fn web_socket_debugger_url(shown: &str, body: &str) -> crate::Result<String> {
    let version: Value = serde_json::from_str(body).map_err(|error| {
        connect_error(format!(
            "The /json/version reply of {shown} is not JSON ({error})"
        ))
    })?;
    version
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            connect_error(format!(
                "The /json/version reply of {shown} has no webSocketDebuggerUrl"
            ))
        })
}

fn rewrite_to_requested_host(reported: &str, base: &Url, shown: &str) -> crate::Result<String> {
    let invalid = |detail: String| {
        connect_error(format!(
            "The /json/version reply of {shown} has an unusable webSocketDebuggerUrl ({detail})"
        ))
    };
    let mut ws_url = Url::parse(reported).map_err(|error| invalid(error.to_string()))?;
    if !matches!(ws_url.scheme(), "ws" | "wss") {
        return Err(invalid(format!(
            "scheme {} is not ws or wss",
            ws_url.scheme()
        )));
    }
    if base.scheme() == "https" && ws_url.scheme() == "ws" {
        ws_url
            .set_scheme("wss")
            .map_err(|()| invalid("cannot switch it to wss".to_owned()))?;
    }
    ws_url
        .set_host(base.host_str())
        .map_err(|error| invalid(error.to_string()))?;
    ws_url
        .set_port(base.port_or_known_default())
        .map_err(|()| invalid("cannot carry a port".to_owned()))?;
    Ok(ws_url.into())
}

fn parse_active_port(content: &str) -> Result<(u16, String), String> {
    let lines: Vec<&str> = content.trim().lines().map(str::trim).collect();
    let [port, path] = lines.as_slice() else {
        return Err(format!("{} lines instead of 2", lines.len()));
    };
    let port = port
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| format!("port {port:?} is not in 1..=65535"))?;
    if !path.starts_with(BROWSER_WS_PATH) {
        return Err(format!(
            "path {path:?} does not start with {BROWSER_WS_PATH}"
        ));
    }
    Ok((port, (*path).to_owned()))
}

fn webdriver_status_url(url: &str) -> Option<Url> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let mut status_url = Url::parse(&with_http_scheme(url))
        .inspect_err(|error| {
            tracing::debug!("WebDriver probe skipped, the URL is invalid: {error}")
        })
        .ok()?;
    let path = format!("{}/status", status_url.path().trim_end_matches('/'));
    status_url.set_path(&path);
    Some(status_url)
}

fn classify_webdriver_status(body: &[u8]) -> WebDriverProbe {
    let Ok(status) = serde_json::from_slice::<Value>(body) else {
        return WebDriverProbe::NotRunning;
    };
    let value = &status["value"];
    let message = value["message"].as_str().unwrap_or_default();
    if CHROMEDRIVER_MARKERS
        .iter()
        .any(|marker| message.contains(marker))
    {
        return WebDriverProbe::Chromedriver;
    }
    if !value["ready"].is_boolean() {
        return WebDriverProbe::NotRunning;
    }
    let name = if message.is_empty() {
        "WebDriver"
    } else {
        message
    };
    WebDriverProbe::Other {
        name: name.to_owned(),
    }
}

fn has_websocket_scheme(address: &str) -> bool {
    let lowered = address.to_ascii_lowercase();
    lowered.starts_with("ws://") || lowered.starts_with("wss://")
}

fn with_http_scheme(address: &str) -> String {
    if address.contains("://") {
        address.to_owned()
    } else {
        format!("http://{address}")
    }
}

fn is_loopback_host(host: Option<Host<&str>>) -> bool {
    match host {
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn shown_address(address: &str, base: &Url) -> String {
    if base.query().is_none() && base.username().is_empty() && base.password().is_none() {
        address.to_owned()
    } else {
        base.origin().ascii_serialization()
    }
}

fn error_sources<'a>(error: &'a reqwest::Error) -> impl Iterator<Item = &'a (dyn Error + 'static)> {
    let first: &'a (dyn Error + 'static) = error;
    std::iter::successors(Some(first), |current| (*current).source())
}

fn is_connection_refused(error: &reqwest::Error) -> bool {
    error_sources(error).any(|current| {
        current
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::ConnectionRefused)
    })
}

fn error_chain(error: reqwest::Error) -> String {
    let error = error.without_url();
    error_sources(&error)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

fn browser_name(kind: BrowserKind) -> &'static str {
    match kind {
        BrowserKind::Chrome => "Chrome",
        BrowserKind::Edge => "Edge",
    }
}

fn connect_error(message: String) -> BrowserError {
    BrowserError::Connect { message }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(reported: &str, base: &str) -> String {
        let base = Url::parse(base).expect("test base URL parses");
        rewrite_to_requested_host(reported, &base, base.as_str()).expect("rewrite succeeds")
    }

    #[test]
    fn rewrite_takes_host_and_port_from_the_requested_address() {
        assert_eq!(
            rewrite(
                "ws://127.0.0.1:50397/devtools/browser/abc",
                "http://localhost:9222"
            ),
            "ws://localhost:9222/devtools/browser/abc"
        );
        assert_eq!(
            rewrite("ws://127.0.0.1:1/devtools/browser/abc", "http://[::1]:9333"),
            "ws://[::1]:9333/devtools/browser/abc"
        );
    }

    #[test]
    fn rewrite_upgrades_ws_to_wss_for_https_discovery() {
        assert_eq!(
            rewrite(
                "ws://127.0.0.1:9222/devtools/browser/abc",
                "https://cdp.example.com"
            ),
            "wss://cdp.example.com/devtools/browser/abc"
        );
        assert_eq!(
            rewrite(
                "ws://127.0.0.1:9222/devtools/browser/abc",
                "https://cdp.example.com:8443"
            ),
            "wss://cdp.example.com:8443/devtools/browser/abc"
        );
    }

    #[test]
    fn rewrite_rejects_non_websocket_urls() {
        let base = Url::parse("http://127.0.0.1:9222").expect("test base URL parses");
        let error = rewrite_to_requested_host("http://127.0.0.1:9222/x", &base, "127.0.0.1:9222")
            .expect_err("an http URL is not a WebSocket endpoint");
        assert!(
            error.to_string().contains("scheme http is not ws or wss"),
            "{error}"
        );
    }

    #[test]
    fn active_port_parse_is_strict() {
        assert_eq!(
            parse_active_port("50397\n/devtools/browser/6730c76a"),
            Ok((50397, "/devtools/browser/6730c76a".to_owned()))
        );
        assert_eq!(
            parse_active_port("9222\r\n/devtools/browser/x\r\n\n"),
            Ok((9222, "/devtools/browser/x".to_owned()))
        );
        assert!(parse_active_port("9222").is_err());
        assert!(parse_active_port("0\n/devtools/browser/x").is_err());
        assert!(parse_active_port("70000\n/devtools/browser/x").is_err());
        assert!(parse_active_port("9222\n/devtools/page/x").is_err());
        assert!(parse_active_port("9222\n/devtools/browser/x\nextra").is_err());
    }

    #[test]
    fn shown_address_hides_credentials_and_query_tokens() {
        let plain = Url::parse("http://127.0.0.1:9222").expect("parses");
        assert_eq!(shown_address("127.0.0.1:9222", &plain), "127.0.0.1:9222");
        let secret = Url::parse("https://user:pw@cdp.example.com/?token=abc").expect("parses");
        assert_eq!(
            shown_address("https://user:pw@cdp.example.com/?token=abc", &secret),
            "https://cdp.example.com"
        );
    }

    #[tokio::test]
    async fn error_chain_leaves_out_the_request_url_and_its_query() {
        let released = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = released.local_addr().expect("read the bound port").port();
        drop(released);
        let client = http_client(&[], PROBE_TIMEOUT, HttpProxy::None).expect("build the client");
        let error = client
            .get(format!(
                "http://127.0.0.1:{port}/wd/hub/status?token=secret"
            ))
            .send()
            .await
            .expect_err("nothing listens on the released port");
        assert!(error.to_string().contains("token=secret"), "{error}");
        let chain = error_chain(error);
        assert!(!chain.contains("secret"), "{chain}");
        assert!(!chain.contains("/wd/hub/status"), "{chain}");
    }

    #[test]
    fn webdriver_status_url_appends_status_to_the_base_path() {
        let status = |url: &str| webdriver_status_url(url).map(String::from);
        assert_eq!(
            status("http://localhost:9515"),
            Some("http://localhost:9515/status".to_owned())
        );
        assert_eq!(
            status(" localhost:4444/wd/hub/ "),
            Some("http://localhost:4444/wd/hub/status".to_owned())
        );
        assert_eq!(
            status("http://127.0.0.1:4444/wd/hub?x=1"),
            Some("http://127.0.0.1:4444/wd/hub/status?x=1".to_owned())
        );
        assert_eq!(status(""), None);
        assert_eq!(status("http://[::1"), None);
    }
}
