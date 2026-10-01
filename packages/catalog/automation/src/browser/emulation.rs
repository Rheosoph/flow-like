use super::content::page_node;
use crate::types::handles::Geolocation;
#[cfg(any(feature = "execute", test))]
use crate::types::handles::{AutomationSession, BrowserType};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(any(feature = "execute", test))]
use flow_like_types::Value;
use flow_like_types::{async_trait, json::json};

#[cfg(any(feature = "execute", test))]
pub(crate) fn is_chromium(session: &AutomationSession) -> bool {
    matches!(
        session.browser_type,
        Some(BrowserType::Chrome | BrowserType::Edge)
    )
}

/// Fails with a readable error before a Chrome DevTools command reaches another browser.
#[cfg(any(feature = "execute", test))]
pub(crate) fn require_chromium(
    session: &AutomationSession,
    feature: &str,
) -> flow_like_types::Result<()> {
    match &session.browser_type {
        _ if is_chromium(session) => Ok(()),
        Some(other) => Err(flow_like_types::anyhow!(
            "{feature} requires Chrome or Edge; this session runs {other:?}"
        )),
        None => Err(flow_like_types::anyhow!(
            "{feature} requires an attached Chrome or Edge browser"
        )),
    }
}

const COLOR_SCHEMES: [&str; 4] = ["", "light", "dark", "no-preference"];

/// Largest viewport dimension Chrome accepts for device metrics overrides.
#[cfg(any(feature = "execute", test))]
const MAX_VIEWPORT_SIDE: i64 = 10_000_000;

#[cfg(any(feature = "execute", test))]
fn device_metrics_params(
    width: i64,
    height: i64,
    device_scale_factor: f64,
    mobile: bool,
) -> flow_like_types::Result<Option<Value>> {
    if width == 0 && height == 0 {
        return Ok(None);
    }
    if !(1..=MAX_VIEWPORT_SIDE).contains(&width) || !(1..=MAX_VIEWPORT_SIDE).contains(&height) {
        return Err(flow_like_types::anyhow!(
            "Viewport width and height must both be between 1 and {MAX_VIEWPORT_SIDE}, got {width}×{height}"
        ));
    }
    if !device_scale_factor.is_finite() || device_scale_factor < 0.0 {
        return Err(flow_like_types::anyhow!(
            "Device scale factor must be 0 (keep) or positive, got {device_scale_factor}"
        ));
    }
    Ok(Some(json!({
        "width": width,
        "height": height,
        "deviceScaleFactor": device_scale_factor,
        "mobile": mobile,
    })))
}

#[cfg(any(feature = "execute", test))]
fn geolocation_params(geolocation: &Geolocation) -> flow_like_types::Result<Value> {
    let Geolocation {
        latitude,
        longitude,
        accuracy,
    } = *geolocation;
    let accuracy = accuracy.unwrap_or(0.0);
    if !latitude.is_finite() || !(-90.0..=90.0).contains(&latitude) {
        return Err(flow_like_types::anyhow!(
            "Latitude must be between -90 and 90, got {latitude}"
        ));
    }
    if !longitude.is_finite() || !(-180.0..=180.0).contains(&longitude) {
        return Err(flow_like_types::anyhow!(
            "Longitude must be between -180 and 180, got {longitude}"
        ));
    }
    if !accuracy.is_finite() || accuracy < 0.0 {
        return Err(flow_like_types::anyhow!(
            "Geolocation accuracy must be a nonnegative number of metres, got {accuracy}"
        ));
    }
    Ok(json!({"latitude": latitude, "longitude": longitude, "accuracy": accuracy}))
}

#[cfg(any(feature = "execute", test))]
struct EmulationSettings {
    locale: String,
    timezone_id: String,
    geolocation: Option<Geolocation>,
    viewport_width: i64,
    viewport_height: i64,
    device_scale_factor: f64,
    mobile: bool,
    color_scheme: String,
    user_agent: String,
    offline: bool,
}

#[cfg(any(feature = "execute", test))]
impl EmulationSettings {
    /// A locale also sets Accept-Language, which CDP only accepts together with a user agent.
    fn needs_current_user_agent(&self) -> bool {
        self.user_agent.trim().is_empty() && !self.locale.trim().is_empty()
    }

    /// Chrome DevTools commands, in order, that apply these settings.
    fn commands(
        &self,
        current_user_agent: &str,
    ) -> flow_like_types::Result<Vec<(&'static str, Value)>> {
        let locale = self.locale.trim();
        let timezone_id = self.timezone_id.trim();
        let user_agent = match self.user_agent.trim() {
            "" => current_user_agent,
            user_agent => user_agent,
        };
        if !COLOR_SCHEMES.contains(&self.color_scheme.as_str()) {
            return Err(flow_like_types::anyhow!(
                "Color scheme must be light, dark, no-preference, or empty, got '{}'",
                self.color_scheme
            ));
        }
        let mut commands = Vec::new();
        if !locale.is_empty() {
            commands.push(("Emulation.setLocaleOverride", json!({"locale": locale})));
        }
        if !timezone_id.is_empty() {
            commands.push((
                "Emulation.setTimezoneOverride",
                json!({"timezoneId": timezone_id}),
            ));
        }
        if let Some(geolocation) = &self.geolocation {
            let position = geolocation_params(geolocation)?;
            commands.push((
                "Browser.grantPermissions",
                json!({"permissions": ["geolocation"]}),
            ));
            commands.push(("Emulation.setGeolocationOverride", position));
        }
        if let Some(metrics) = device_metrics_params(
            self.viewport_width,
            self.viewport_height,
            self.device_scale_factor,
            self.mobile,
        )? {
            commands.push(("Emulation.setDeviceMetricsOverride", metrics));
        }
        if !self.color_scheme.is_empty() {
            commands.push((
                "Emulation.setEmulatedMedia",
                json!({"features": [{"name": "prefers-color-scheme", "value": self.color_scheme}]}),
            ));
        }
        if !user_agent.is_empty() {
            let mut params = json!({"userAgent": user_agent});
            if !locale.is_empty() {
                params["acceptLanguage"] = json!(locale);
            }
            commands.push(("Emulation.setUserAgentOverride", params));
        }
        commands.push(("Network.enable", json!({})));
        commands.push((
            "Network.emulateNetworkConditions",
            json!({"offline": self.offline, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1}),
        ));
        Ok(commands)
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserSetEmulationNode {}

impl BrowserSetEmulationNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserSetEmulationNode {
    fn get_node(&self) -> Node {
        let mut node = page_node(
            "browser_set_emulation",
            "Set Emulation",
            "Makes the current tab behave like another device or place: locale, time zone, geolocation, screen size, color scheme, user agent and offline mode. Empty or zero inputs leave that setting unchanged; Offline is always applied. Overrides last until the tab closes. Chrome and Edge only.",
            "Emulation",
        );
        node.set_flowscript_name("browser", "setEmulation");
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(7)
                .set_performance(9)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "locale",
            "Locale",
            "BCP 47 locale such as de-DE, used for Intl formatting, navigator.language and the Accept-Language header",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "timezone_id",
            "Time Zone",
            "IANA time zone such as Europe/Berlin",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "geolocation",
            "Geolocation",
            "Position reported by the Geolocation API; also grants the geolocation permission",
            VariableType::Struct,
        )
        .set_schema::<Geolocation>()
        .set_options(PinOptions::new().set_optional(true).build())
        .set_default_value(Some(json!(null)));
        node.add_input_pin(
            "viewport_width",
            "Viewport Width",
            "Emulated screen width in CSS pixels; set together with Viewport Height",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "viewport_height",
            "Viewport Height",
            "Emulated screen height in CSS pixels; set together with Viewport Width",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "device_scale_factor",
            "Device Scale Factor",
            "Device pixel ratio for the emulated screen; 0 keeps the display's own ratio",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "mobile",
            "Mobile",
            "Emulate a mobile device (meta viewport, overlay scrollbars) with the viewport size",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "color_scheme",
            "Color Scheme",
            "Value reported to prefers-color-scheme media queries",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(COLOR_SCHEMES.map(str::to_string).to_vec())
                .build(),
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "user_agent",
            "User Agent",
            "User-Agent header and navigator.userAgent for this tab",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "offline",
            "Offline",
            "Simulate a lost network connection; false restores connectivity",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        require_chromium(&session, "Browser emulation")?;
        let settings = EmulationSettings {
            locale: context.evaluate_pin("locale").await?,
            timezone_id: context.evaluate_pin("timezone_id").await?,
            geolocation: context.evaluate_pin("geolocation").await?,
            viewport_width: context.evaluate_pin("viewport_width").await?,
            viewport_height: context.evaluate_pin("viewport_height").await?,
            device_scale_factor: context.evaluate_pin("device_scale_factor").await?,
            mobile: context.evaluate_pin("mobile").await?,
            color_scheme: context.evaluate_pin("color_scheme").await?,
            user_agent: context.evaluate_pin("user_agent").await?,
            offline: context.evaluate_pin("offline").await?,
        };
        settings.commands("")?;

        let page = session.browser_page(context).await?;
        let current_user_agent = if settings.needs_current_user_agent() {
            page.probe("return navigator.userAgent;", vec![])
                .await?
                .json()
                .as_str()
                .unwrap_or_default()
                .to_owned()
        } else {
            String::new()
        };
        for (method, params) in settings.commands(&current_user_agent)? {
            super::cdp::send(&page, method, params).await?;
        }
        drop(page);

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

#[cfg(any(feature = "execute", test))]
fn is_header_token(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

/// Validates a header name → value object for `Network.setExtraHTTPHeaders`.
#[cfg(any(feature = "execute", test))]
fn extra_headers(
    headers: &Value,
) -> flow_like_types::Result<flow_like_types::json::Map<String, Value>> {
    let headers = match headers {
        Value::Null => return Ok(Default::default()),
        Value::Object(headers) => headers,
        other => {
            return Err(flow_like_types::anyhow!(
                "Extra headers must be an object of header names to string values, got {}",
                if other.is_array() {
                    "an array"
                } else {
                    "a scalar"
                }
            ));
        }
    };
    headers
        .iter()
        .map(|(name, value)| {
            if !is_header_token(name) {
                return Err(flow_like_types::anyhow!(
                    "'{name}' is not a valid HTTP header name"
                ));
            }
            let value = value.as_str().ok_or_else(|| {
                flow_like_types::anyhow!("Header '{name}' must have a string value")
            })?;
            if value.contains(['\r', '\n', '\0']) {
                return Err(flow_like_types::anyhow!(
                    "Header '{name}' value must not contain line breaks or NUL"
                ));
            }
            Ok((name.clone(), json!(value)))
        })
        .collect()
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserSetExtraHeadersNode {}

impl BrowserSetExtraHeadersNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserSetExtraHeadersNode {
    fn get_node(&self) -> Node {
        let mut node = page_node(
            "browser_set_extra_headers",
            "Set Extra Headers",
            "Adds HTTP headers to every request the current tab makes, including requests to third-party origins, so avoid credentials on pages that load foreign content. Replaces headers set earlier; an empty object removes them. Chrome and Edge only.",
            "Emulation",
        );
        node.set_flowscript_name("browser", "setExtraHeaders");
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(4)
                .set_performance(9)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "headers",
            "Headers",
            "Object mapping header names to string values, e.g. {\"X-Tenant\": \"acme\"}",
            VariableType::Struct,
        )
        .set_open_schema()
        .set_options(PinOptions::new().set_sensitive(true).build())
        .set_default_value(Some(json!({})));
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::cdp::send;
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        require_chromium(&session, "Extra HTTP headers")?;
        let headers: Value = context.evaluate_pin("headers").await?;
        let headers = extra_headers(&headers)?;

        let page = session.browser_page(context).await?;
        send(&page, "Network.enable", json!({})).await?;
        send(
            &page,
            "Network.setExtraHTTPHeaders",
            json!({"headers": headers}),
        )
        .await?;
        drop(page);

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

#[cfg(any(feature = "execute", test))]
fn blocked_url_patterns(patterns: Vec<String>) -> flow_like_types::Result<Vec<String>> {
    patterns
        .into_iter()
        .map(|pattern| pattern.trim().to_owned())
        .filter(|pattern| !pattern.is_empty())
        .map(|pattern| {
            if pattern.chars().any(char::is_whitespace) {
                Err(flow_like_types::anyhow!(
                    "Blocked URL pattern '{pattern}' must not contain whitespace"
                ))
            } else {
                Ok(pattern)
            }
        })
        .collect()
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserBlockUrlsNode {}

impl BrowserBlockUrlsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserBlockUrlsNode {
    fn get_node(&self) -> Node {
        let mut node = page_node(
            "browser_block_urls",
            "Block URLs",
            "Stops the current tab from loading matching URLs, such as ads, trackers or heavy media. Patterns use * as a wildcard. Replaces the previous list; an empty list unblocks everything. Chrome and Edge only.",
            "Emulation",
        );
        node.set_flowscript_name("browser", "blockUrls");
        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "patterns",
            "Patterns",
            "URL patterns such as *://*.doubleclick.net/* or *.mp4",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));
        node.add_output_pin(
            "blocked_count",
            "Blocked Patterns",
            "Number of patterns now blocked",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::cdp::send;
        context.check_cancelled()?;
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        require_chromium(&session, "URL blocking")?;
        let patterns = blocked_url_patterns(context.evaluate_pin("patterns").await?)?;

        let page = session.browser_page(context).await?;
        send(&page, "Network.enable", json!({})).await?;
        send(&page, "Network.setBlockedURLs", json!({"urls": patterns})).await?;
        drop(page);

        context
            .set_pin_value("blocked_count", json!(patterns.len() as i64))
            .await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn session(browser_type: Option<BrowserType>) -> AutomationSession {
        AutomationSession {
            session_ref: "s".into(),
            platform: crate::types::handles::Platform::Linux,
            default_delay_ms: 0,
            click_delay_ms: 0,
            debug_mode: false,
            browser_type,
            browser_headless: None,
            browser_user_data_dir: None,
            current_page_ref: None,
            current_window_handle: None,
            browser_frame_selectors: Vec::new(),
        }
    }

    #[test]
    fn devtools_features_are_limited_to_chromium() {
        assert!(require_chromium(&session(Some(BrowserType::Edge)), "X").is_ok());
        let firefox = require_chromium(&session(Some(BrowserType::Firefox)), "Emulation")
            .unwrap_err()
            .to_string();
        assert!(firefox.contains("Emulation") && firefox.contains("Firefox"));
        assert!(require_chromium(&session(None), "X").is_err());
    }

    #[test]
    fn device_metrics_need_both_sides() {
        assert_eq!(device_metrics_params(0, 0, 0.0, false).unwrap(), None);
        assert_eq!(
            device_metrics_params(390, 844, 3.0, true).unwrap(),
            Some(json!({"width": 390, "height": 844, "deviceScaleFactor": 3.0, "mobile": true}))
        );
        assert!(device_metrics_params(390, 0, 0.0, false).is_err());
        assert!(device_metrics_params(-1, 10, 0.0, false).is_err());
        assert!(device_metrics_params(10, 10, -2.0, false).is_err());
    }

    fn settings() -> EmulationSettings {
        EmulationSettings {
            locale: String::new(),
            timezone_id: String::new(),
            geolocation: None,
            viewport_width: 0,
            viewport_height: 0,
            device_scale_factor: 0.0,
            mobile: false,
            color_scheme: String::new(),
            user_agent: String::new(),
            offline: false,
        }
    }

    #[test]
    fn empty_settings_only_restore_connectivity() {
        let commands = settings().commands("").unwrap();
        let methods: Vec<_> = commands.iter().map(|(method, _)| *method).collect();
        assert_eq!(
            methods,
            ["Network.enable", "Network.emulateNetworkConditions"]
        );
        assert_eq!(commands[1].1["offline"], json!(false));
    }

    #[test]
    fn locale_sets_accept_language_with_the_current_user_agent() {
        let settings = EmulationSettings {
            locale: " de-DE ".into(),
            timezone_id: "Europe/Berlin".into(),
            color_scheme: "dark".into(),
            geolocation: Some(Geolocation {
                latitude: 52.5,
                longitude: 13.4,
                accuracy: Some(10.0),
            }),
            offline: true,
            ..settings()
        };
        assert!(settings.needs_current_user_agent());
        let commands = settings.commands("Mozilla/5.0 Test").unwrap();
        let find = |name: &str| {
            commands
                .iter()
                .find(|(method, _)| *method == name)
                .map(|(_, params)| params.clone())
                .unwrap_or_else(|| panic!("missing {name}"))
        };
        assert_eq!(
            find("Emulation.setLocaleOverride"),
            json!({"locale": "de-DE"})
        );
        assert_eq!(
            find("Emulation.setUserAgentOverride"),
            json!({"userAgent": "Mozilla/5.0 Test", "acceptLanguage": "de-DE"})
        );
        assert_eq!(
            find("Emulation.setTimezoneOverride"),
            json!({"timezoneId": "Europe/Berlin"})
        );
        assert_eq!(
            find("Emulation.setEmulatedMedia")["features"][0]["value"],
            json!("dark")
        );
        assert_eq!(
            find("Emulation.setGeolocationOverride")["accuracy"],
            json!(10.0)
        );
        let grant = commands
            .iter()
            .position(|(method, _)| *method == "Browser.grantPermissions")
            .unwrap();
        assert_eq!(commands[grant + 1].0, "Emulation.setGeolocationOverride");
        assert_eq!(
            find("Network.emulateNetworkConditions")["offline"],
            json!(true)
        );
    }

    #[test]
    fn invalid_settings_fail_before_any_command() {
        let bad_scheme = EmulationSettings {
            color_scheme: "sepia".into(),
            ..settings()
        };
        assert!(bad_scheme.commands("").is_err());
        let half_viewport = EmulationSettings {
            viewport_width: 800,
            ..settings()
        };
        assert!(half_viewport.commands("").is_err());
        let explicit_agent = EmulationSettings {
            user_agent: "Bot/1.0".into(),
            ..settings()
        };
        assert!(!explicit_agent.needs_current_user_agent());
        assert!(
            explicit_agent
                .commands("ignored")
                .unwrap()
                .iter()
                .any(
                    |(method, params)| *method == "Emulation.setUserAgentOverride"
                        && params == &json!({"userAgent": "Bot/1.0"})
                )
        );
    }

    #[test]
    fn geolocation_is_range_checked() {
        let berlin = Geolocation {
            latitude: 52.52,
            longitude: 13.405,
            accuracy: None,
        };
        assert_eq!(
            geolocation_params(&berlin).unwrap(),
            json!({"latitude": 52.52, "longitude": 13.405, "accuracy": 0.0})
        );
        for (latitude, longitude, accuracy) in [
            (91.0, 0.0, None),
            (0.0, 181.0, None),
            (0.0, 0.0, Some(-1.0)),
        ] {
            assert!(
                geolocation_params(&Geolocation {
                    latitude,
                    longitude,
                    accuracy
                })
                .is_err()
            );
        }
    }

    #[test]
    fn extra_headers_reject_injection() {
        let headers =
            extra_headers(&json!({"X-Tenant": "acme", "Authorization": "Bearer t"})).unwrap();
        assert_eq!(headers.len(), 2);
        assert!(extra_headers(&json!({})).unwrap().is_empty());
        assert!(extra_headers(&Value::Null).unwrap().is_empty());
        assert!(extra_headers(&json!({"X-A": "ok\r\nSet-Cookie: x"})).is_err());
        assert!(extra_headers(&json!({"Bad Name": "v"})).is_err());
        assert!(extra_headers(&json!({"X-Num": 5})).is_err());
        assert!(extra_headers(&json!(["X-A", "b"])).is_err());
    }

    #[test]
    fn blocked_patterns_drop_blanks() {
        assert_eq!(
            blocked_url_patterns(vec![
                " *.mp4 ".into(),
                "".into(),
                "*://ads.example/*".into()
            ])
            .unwrap(),
            vec!["*.mp4".to_string(), "*://ads.example/*".to_string()]
        );
        assert!(blocked_url_patterns(vec![]).unwrap().is_empty());
        assert!(blocked_url_patterns(vec!["a b".into()]).is_err());
    }
}
