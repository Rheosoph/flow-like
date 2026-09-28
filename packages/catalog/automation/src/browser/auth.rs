use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct BasicAuthCredentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for BasicAuthCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BasicAuthCredentials")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
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

impl From<super::persist::StorageCookie> for CookieData {
    fn from(cookie: super::persist::StorageCookie) -> Self {
        Self {
            expiry: (cookie.expires > 0.0).then_some(cookie.expires as i64),
            name: cookie.name,
            value: cookie.value,
            domain: Some(cookie.domain).filter(|domain| !domain.is_empty()),
            path: Some(cookie.path),
            secure: Some(cookie.secure),
            http_only: Some(cookie.http_only),
            same_site: Some(cookie.same_site),
        }
    }
}

impl From<CookieData> for super::persist::StorageCookie {
    fn from(cookie: CookieData) -> Self {
        Self {
            name: cookie.name,
            value: cookie.value,
            domain: cookie.domain.unwrap_or_default(),
            path: cookie.path.unwrap_or_else(|| "/".to_string()),
            expires: cookie.expiry.map_or(-1.0, |expiry| expiry as f64),
            http_only: cookie.http_only.unwrap_or(false),
            secure: cookie.secure.unwrap_or(false),
            same_site: cookie.same_site.unwrap_or_else(|| "Lax".to_string()),
        }
    }
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
        node.set_version(1);
        node.set_flowscript_name("browser", "setBasicAuth");
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
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_sensitive(true)
                .build(),
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "origin",
            "Origin",
            "HTTP(S) origin allowed to receive credentials",
            VariableType::String,
        );
        node.add_input_pin(
            "debugger_address",
            "Debugger Address",
            "Optional Chrome or Edge debugger address; defaults to the attached browser",
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

        let origin: String = context.evaluate_pin("origin").await?;
        let debugger_address: String = context.evaluate_pin("debugger_address").await?;
        let origin = super::protocol::normalized_origin(&origin)?;
        super::protocol::start_listener(
            context,
            &session,
            &driver,
            &debugger_address,
            Some((origin, username, password)),
        )
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
            "Saves browser cookies to a file for later restoration: every domain including HttpOnly cookies on Chrome and Edge, the current document's cookies on other browsers",
            "Automation/Browser/Auth",
        );
        node.set_version(2);
        node.set_flowscript_name("browser", "saveCookies");
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
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let file_path: FlowPath = context.evaluate_pin("file_path").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;
        let cookies = super::persist::read_cookies(&session, &driver).await?;
        drop(driver);
        let cookie_data: Vec<CookieData> = cookies.into_iter().map(CookieData::from).collect();

        let cookie_json = flow_like_types::json::to_string_pretty(&cookie_data)
            .map_err(|e| flow_like_types::anyhow!("Failed to serialize cookies: {}", e))?;

        let runtime = file_path.to_runtime(context).await?;
        let store = runtime.store.as_generic();

        store
            .put(&runtime.path, cookie_json.into())
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to save cookies to {}: {}", runtime.path, e))?;

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
            "Loads cookies from a file into the browser session. Chrome and Edge accept cookies for every domain with their HttpOnly, Secure and SameSite flags; other browsers only accept cookies for the current page's domain.",
            "Automation/Browser/Auth",
        );
        node.set_version(2);
        node.set_flowscript_name("browser", "loadCookies");
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
            "Number of cookies the browser accepted",
            VariableType::Integer,
        );

        node.add_output_pin(
            "failed_count",
            "Failed Count",
            "Number of cookies that were expired or rejected",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let file_path: FlowPath = context.evaluate_pin("file_path").await?;

        let runtime = file_path.to_runtime(context).await?;
        let store = runtime.store.as_generic();

        let data = match store.get(&runtime.path).await {
            Ok(data) => data.bytes().await.map_err(|e| {
                flow_like_types::anyhow!("Failed to read cookies file {}: {}", runtime.path, e)
            })?,
            Err(_) => {
                context.set_pin_value("session_out", json!(session)).await?;
                context.set_pin_value("cookie_count", json!(0)).await?;
                super::selector::optional_output(context, "failed_count", json!(0)).await?;
                context.activate_exec_pin("exec_error").await?;
                return Ok(());
            }
        };

        let cookie_data: Vec<CookieData> = flow_like_types::json::from_slice(&data)
            .map_err(|e| flow_like_types::anyhow!("Failed to parse cookies file: {}", e))?;
        let cookies: Vec<super::persist::StorageCookie> =
            cookie_data.into_iter().map(Into::into).collect();

        let driver = session.get_browser_driver_and_switch(context).await?;
        let report = super::persist::write_cookies(context, &session, &driver, &cookies).await?;
        drop(driver);

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("cookie_count", json!(report.applied))
            .await?;
        super::selector::optional_output(context, "failed_count", json!(report.failed)).await?;
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
            "Clears cookies: every domain on Chrome and Edge, the current document's cookies on other browsers",
            "Automation/Browser/Auth",
        );
        node.set_version(2);
        node.set_flowscript_name("browser", "clearCookies");
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
        let driver = session.get_browser_driver_and_switch(context).await?;
        super::persist::clear_cookies(&session, &driver).await?;
        drop(driver);

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TotpAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

#[cfg(any(feature = "execute", test))]
impl TotpAlgorithm {
    fn parse(value: &str) -> flow_like_types::Result<Self> {
        match value.trim().to_ascii_uppercase().replace('-', "").as_str() {
            "SHA1" => Ok(Self::Sha1),
            "SHA256" => Ok(Self::Sha256),
            "SHA512" => Ok(Self::Sha512),
            _ => Err(flow_like_types::anyhow!(
                "Unknown TOTP algorithm '{value}' (use SHA1, SHA256 or SHA512)"
            )),
        }
    }

    fn mac(self, key: &[u8], message: &[u8]) -> Vec<u8> {
        use hmac::{Mac, SimpleHmac};
        fn sign<D: hmac::digest::Digest + hmac::digest::core_api::BlockSizeUser>(
            key: &[u8],
            message: &[u8],
        ) -> Vec<u8> {
            let mut mac = <SimpleHmac<D> as Mac>::new_from_slice(key)
                .expect("HMAC accepts keys of any length");
            mac.update(message);
            mac.finalize().into_bytes().to_vec()
        }
        match self {
            Self::Sha1 => sign::<sha1::Sha1>(key, message),
            Self::Sha256 => sign::<sha2::Sha256>(key, message),
            Self::Sha512 => sign::<sha2::Sha512>(key, message),
        }
    }
}

/// RFC 4648 base32 (case-insensitive; spaces, dashes and padding ignored).
#[cfg(any(feature = "execute", test))]
fn decode_base32(secret: &str) -> flow_like_types::Result<Vec<u8>> {
    let mut buffer: u32 = 0;
    let mut bits = 0;
    let mut bytes = Vec::new();
    for (position, character) in secret
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '-' | '='))
        .enumerate()
    {
        let value = match character.to_ascii_uppercase() {
            letter @ 'A'..='Z' => letter as u32 - 'A' as u32,
            digit @ '2'..='7' => digit as u32 - '2' as u32 + 26,
            _ => {
                return Err(flow_like_types::anyhow!(
                    "TOTP secret is not valid base32 (unexpected character at position {})",
                    position + 1
                ));
            }
        };
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    if bytes.is_empty() {
        return Err(flow_like_types::anyhow!("TOTP secret is empty"));
    }
    Ok(bytes)
}

/// RFC 6238 code for `unix_time` and the seconds until it changes.
#[cfg(any(feature = "execute", test))]
fn totp(
    key: &[u8],
    unix_time: u64,
    period: u64,
    digits: u32,
    algorithm: TotpAlgorithm,
) -> flow_like_types::Result<(String, u64)> {
    if !(6..=8).contains(&digits) {
        return Err(flow_like_types::anyhow!(
            "TOTP digits must be 6, 7 or 8 (got {digits})"
        ));
    }
    if period == 0 {
        return Err(flow_like_types::anyhow!("TOTP period must be positive"));
    }
    let hash = algorithm.mac(key, &(unix_time / period).to_be_bytes());
    let offset = usize::from(hash[hash.len() - 1] & 0x0f);
    let binary = u32::from_be_bytes([
        hash[offset] & 0x7f,
        hash[offset + 1],
        hash[offset + 2],
        hash[offset + 3],
    ]);
    let code = binary % 10u32.pow(digits);
    Ok((
        format!("{code:0width$}", width = digits as usize),
        period - unix_time % period,
    ))
}

#[crate::register_node]
#[derive(Default)]
pub struct TotpCodeNode {}

impl TotpCodeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for TotpCodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "totp_code",
            "TOTP Code",
            "Generates the current RFC 6238 one-time password (authenticator app code) from a base32 secret",
            "Automation/Browser/Auth",
        );
        node.set_version(1);
        node.set_flowscript_name("browser", "totpCode");
        node.add_icon("/flow/icons/key.svg");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(10)
                .set_governance(7)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "secret",
            "Secret",
            "Base32 shared secret from the authenticator setup (spaces and dashes are ignored)",
            VariableType::String,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_sensitive(true)
                .build(),
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "digits",
            "Digits",
            "Code length (6 to 8)",
            VariableType::Integer,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_range((6.0, 8.0))
                .build(),
        )
        .set_default_value(Some(json!(6)));
        node.add_input_pin(
            "period",
            "Period (s)",
            "Seconds each code is valid",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));
        node.add_input_pin(
            "algorithm",
            "Algorithm",
            "HMAC algorithm of the secret",
            VariableType::String,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(vec![
                    "SHA1".to_string(),
                    "SHA256".to_string(),
                    "SHA512".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("SHA1")));
        node.add_input_pin(
            "unix_time",
            "Unix Time",
            "Time in Unix seconds to generate the code for; negative uses the current time",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));
        node.add_output_pin("code", "Code", "One-time password", VariableType::String);
        node.add_output_pin(
            "seconds_remaining",
            "Seconds Remaining",
            "Seconds until the code changes",
            VariableType::Integer,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let secret: String = context.evaluate_pin("secret").await?;
        let digits: i64 = context.evaluate_pin("digits").await?;
        let period: i64 = context.evaluate_pin("period").await?;
        let algorithm: String = context.evaluate_pin("algorithm").await?;
        let unix_time: i64 = context.evaluate_pin("unix_time").await?;
        let period = u64::try_from(period)
            .ok()
            .filter(|period| *period > 0)
            .ok_or_else(|| flow_like_types::anyhow!("TOTP period must be positive (got {period})"))?;
        let digits = u32::try_from(digits)
            .map_err(|_| flow_like_types::anyhow!("TOTP digits must be 6, 7 or 8 (got {digits})"))?;
        let unix_time = match u64::try_from(unix_time) {
            Ok(time) => time,
            Err(_) => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
        };
        let key = decode_base32(&secret)?;
        let (code, remaining) =
            totp(&key, unix_time, period, digits, TotpAlgorithm::parse(&algorithm)?)?;
        context.set_pin_value("code", json!(code)).await?;
        context
            .set_pin_value("seconds_remaining", json!(remaining))
            .await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "TOTP generation requires the 'execute' feature"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMES: [u64; 6] = [59, 1_111_111_109, 1_111_111_111, 1_234_567_890, 2_000_000_000, 20_000_000_000];

    fn codes(key: &[u8], algorithm: TotpAlgorithm) -> Vec<String> {
        TIMES
            .iter()
            .map(|time| totp(key, *time, 30, 8, algorithm).unwrap().0)
            .collect()
    }

    #[test]
    fn rfc6238_vectors() {
        assert_eq!(
            codes(b"12345678901234567890", TotpAlgorithm::Sha1),
            ["94287082", "07081804", "14050471", "89005924", "69279037", "65353130"]
        );
        assert_eq!(
            codes(b"12345678901234567890123456789012", TotpAlgorithm::Sha256),
            ["46119246", "68084774", "67062674", "91819424", "90698825", "77737706"]
        );
        assert_eq!(
            codes(
                b"1234567890123456789012345678901234567890123456789012345678901234",
                TotpAlgorithm::Sha512
            ),
            ["90693936", "25091201", "99943326", "93441116", "38618901", "47863826"]
        );
    }

    #[test]
    fn six_digit_codes_and_remaining_seconds() {
        let key = decode_base32("GEZD GNBV-GY3T QOJQ gezd gnbv gy3t qojq====").unwrap();
        assert_eq!(key, b"12345678901234567890");
        assert_eq!(
            totp(&key, 59, 30, 6, TotpAlgorithm::Sha1).unwrap(),
            ("287082".to_string(), 1)
        );
        assert_eq!(totp(&key, 60, 30, 6, TotpAlgorithm::Sha1).unwrap().1, 30);
        assert!(totp(&key, 59, 30, 5, TotpAlgorithm::Sha1).is_err());
        assert!(totp(&key, 59, 0, 6, TotpAlgorithm::Sha1).is_err());
    }

    #[test]
    fn secrets_and_algorithms_are_validated() {
        assert!(decode_base32("").is_err());
        let error = decode_base32("ABC1").unwrap_err().to_string();
        assert!(error.contains("position 4"));
        assert!(!error.contains('1'));
        assert_eq!(TotpAlgorithm::parse("sha-256").unwrap(), TotpAlgorithm::Sha256);
        assert!(TotpAlgorithm::parse("md5").is_err());
    }

    #[test]
    fn cookie_files_keep_http_only() {
        let cookie = super::super::persist::StorageCookie {
            name: "sid".into(),
            value: "v".into(),
            domain: String::new(),
            path: "/".into(),
            expires: -1.0,
            http_only: true,
            secure: true,
            same_site: "Strict".into(),
        };
        let data = CookieData::from(cookie.clone());
        assert_eq!(data.http_only, Some(true));
        assert_eq!(data.domain, None);
        assert_eq!(data.expiry, None);
        assert_eq!(super::super::persist::StorageCookie::from(data), cookie);
        let legacy: CookieData = flow_like_types::json::from_value(json!({
            "name": "a", "value": "b", "domain": ".x.com", "path": null, "secure": null,
            "http_only": null, "same_site": null, "expiry": 1_900_000_000i64
        }))
        .unwrap();
        let restored = super::super::persist::StorageCookie::from(legacy);
        assert_eq!(restored.path, "/");
        assert_eq!(restored.same_site, "Lax");
        assert_eq!(restored.expires, 1_900_000_000.0);
    }
}
