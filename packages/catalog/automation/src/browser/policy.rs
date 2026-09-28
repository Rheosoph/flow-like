#[cfg(feature = "execute")]
use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(any(feature = "execute", test))]
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Navigation rules for one automation session.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq, Eq)]
pub struct NavigationPolicy {
    /// Allowed URL schemes such as `http` and `https`. When empty, every scheme except the
    /// privileged ones (file, javascript, chrome, chrome-untrusted, edge, devtools,
    /// view-source) is allowed; privileged schemes are only allowed when listed.
    #[serde(default)]
    pub allowed_schemes: Vec<String>,
    /// Host globs such as `example.com` (that host only) or `*.example.com` (its subdomains).
    /// Empty allows any host. URLs without a host (about:, data:) are governed by schemes only.
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Host globs that are always blocked; checked before the allowlist.
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    /// Blocks hosts that are, or resolve to, loopback, private, link-local, CGNAT, multicast
    /// or cloud metadata addresses. Hosts that cannot be resolved are blocked too.
    #[serde(default)]
    pub block_private_networks: bool,
}

#[cfg(any(feature = "execute", test))]
const PRIVILEGED_SCHEMES: &[&str] = &[
    "file",
    "javascript",
    "chrome",
    "chrome-untrusted",
    "edge",
    "devtools",
    "view-source",
];

#[cfg(any(feature = "execute", test))]
const METADATA_HOSTS: &[&str] = &["metadata.google.internal", "metadata.goog", "metadata"];

#[cfg(any(feature = "execute", test))]
type Url = flow_like_types::reqwest::Url;

#[cfg(any(feature = "execute", test))]
impl NavigationPolicy {
    /// Browser Go To without a session policy: everything except privileged schemes.
    pub(crate) fn goto_default() -> Self {
        Self::default()
    }

    /// Plan navigation without a session policy: HTTP(S) only.
    pub(crate) fn plan_default() -> Self {
        Self {
            allowed_schemes: vec!["http".into(), "https".into()],
            ..Self::default()
        }
    }

    pub(crate) fn parse(url: &str) -> flow_like_types::Result<Url> {
        Url::parse(url.trim()).map_err(|error| {
            flow_like_types::anyhow!("Navigation to '{url}' blocked: not an absolute URL ({error})")
        })
    }

    /// Scheme, domain list and IP-literal checks without DNS.
    pub(crate) fn check_static(&self, url: &Url) -> flow_like_types::Result<()> {
        let blocked = |reason: String| {
            Err(flow_like_types::anyhow!(
                "Navigation to '{url}' blocked by the navigation policy: {reason}"
            ))
        };
        let scheme = url.scheme().to_ascii_lowercase();
        let listed = self
            .allowed_schemes
            .iter()
            .any(|allowed| allowed.trim().trim_end_matches(':').eq_ignore_ascii_case(&scheme));
        if self.allowed_schemes.is_empty() {
            if PRIVILEGED_SCHEMES.contains(&scheme.as_str()) {
                return blocked(format!(
                    "the '{scheme}:' scheme is blocked unless Set Navigation Policy allows it"
                ));
            }
        } else if !listed {
            return blocked(format!(
                "the '{scheme}:' scheme is not in the allowed schemes ({})",
                self.allowed_schemes.join(", ")
            ));
        }
        let Some(host) = policy_host(url) else {
            return Ok(());
        };
        if let Some(pattern) = self
            .blocked_domains
            .iter()
            .find(|pattern| host_matches(pattern, &host))
        {
            return blocked(format!("host '{host}' matches blocked domain '{pattern}'"));
        }
        if !self.allowed_domains.is_empty()
            && !self
                .allowed_domains
                .iter()
                .any(|pattern| host_matches(pattern, &host))
        {
            return blocked(format!("host '{host}' is not in the allowed domains"));
        }
        if self.block_private_networks {
            if let Ok(ip) = host.parse::<IpAddr>()
                && is_private_ip(ip)
            {
                return blocked(format!("'{ip}' is a private or local network address"));
            }
            if host == "localhost" || host.ends_with(".localhost") || METADATA_HOSTS.contains(&host.as_str()) {
                return blocked(format!("'{host}' is a local or metadata host"));
            }
        }
        Ok(())
    }
}

#[cfg(feature = "execute")]
impl NavigationPolicy {
    /// Full check including DNS resolution when private networks are blocked.
    pub(crate) async fn check(&self, url: &str) -> flow_like_types::Result<Url> {
        let parsed = Self::parse(url)?;
        self.check_static(&parsed)?;
        let Some(domain) = parsed
            .domain()
            .filter(|_| self.block_private_networks)
            .map(str::to_owned)
        else {
            return Ok(parsed);
        };
        let port = parsed.port_or_known_default().unwrap_or(80);
        let lookup = flow_like_types::tokio::time::timeout(
            std::time::Duration::from_secs(5),
            flow_like_types::tokio::net::lookup_host((domain.as_str(), port)),
        )
        .await;
        let Ok(Ok(addresses)) = lookup else {
            return Err(flow_like_types::anyhow!(
                "Navigation to '{url}' blocked by the navigation policy: host '{domain}' could not be resolved to verify it is not a private address"
            ));
        };
        if let Some(address) = addresses.map(|address| address.ip()).find(|ip| is_private_ip(*ip)) {
            return Err(flow_like_types::anyhow!(
                "Navigation to '{url}' blocked by the navigation policy: host '{domain}' resolves to private address {address}"
            ));
        }
        Ok(parsed)
    }
}

#[cfg(any(feature = "execute", test))]
fn policy_host(url: &Url) -> Option<String> {
    let host = url.host_str()?.trim_start_matches('[').trim_end_matches(']');
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

#[cfg(any(feature = "execute", test))]
fn normalize_pattern(pattern: &str) -> String {
    let pattern = pattern.trim();
    let pattern = pattern.split_once("://").map_or(pattern, |(_, rest)| rest);
    let pattern = pattern.split('/').next().unwrap_or_default();
    let pattern = if pattern.starts_with('[') {
        pattern.split(']').next().unwrap_or_default().trim_start_matches('[')
    } else if pattern.matches(':').count() == 1 {
        pattern.split(':').next().unwrap_or_default()
    } else {
        pattern
    };
    pattern.trim_end_matches('.').to_ascii_lowercase()
}

/// Case-insensitive host glob where `*` matches any run of characters.
#[cfg(any(feature = "execute", test))]
pub(crate) fn host_matches(pattern: &str, host: &str) -> bool {
    let pattern: Vec<char> = normalize_pattern(pattern).chars().collect();
    let host: Vec<char> = host.to_ascii_lowercase().chars().collect();
    if pattern.is_empty() {
        return false;
    }
    let (mut p, mut h) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while h < host.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, h));
            p += 1;
        } else if p < pattern.len() && pattern[p] == host[h] {
            p += 1;
            h += 1;
        } else if let Some((star_p, star_h)) = star {
            p = star_p + 1;
            h = star_h + 1;
            star = Some((star_p, star_h + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

#[cfg(any(feature = "execute", test))]
fn private_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || a == 0
        || a >= 240
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (b == 18 || b == 19))
}

#[cfg(any(feature = "execute", test))]
fn embedded_v4(high: u16, low: u16) -> Ipv4Addr {
    let [a, b] = high.to_be_bytes();
    let [c, d] = low.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

#[cfg(any(feature = "execute", test))]
fn private_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return private_v4(v4);
    }
    let segments = ip.segments();
    let ipv4_compatible = segments[..6].iter().all(|segment| *segment == 0);
    let nat64 = segments[0] == 0x64 && segments[1] == 0xff9b && segments[2..6].iter().all(|s| *s == 0);
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        || ((ipv4_compatible || nat64) && private_v4(embedded_v4(segments[6], segments[7])))
        || (segments[0] == 0x2002 && private_v4(embedded_v4(segments[1], segments[2])))
}

/// Loopback, private, link-local (incl. 169.254.169.254), CGNAT (incl. 100.100.100.200),
/// unique-local IPv6 (incl. fd00:ec2::254), multicast, reserved and embedded-IPv4 forms.
#[cfg(any(feature = "execute", test))]
pub(crate) fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => private_v4(ip),
        IpAddr::V6(ip) => private_v6(ip),
    }
}

#[cfg(feature = "execute")]
struct PolicyEntry(NavigationPolicy);

#[cfg(feature = "execute")]
impl flow_like_types::Cacheable for PolicyEntry {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(feature = "execute")]
fn policy_key(session: &AutomationSession) -> String {
    format!("automation:policy:{}", session.session_ref)
}

/// The session's policy, or `fallback` when Set Navigation Policy was not used.
#[cfg(feature = "execute")]
pub(crate) async fn session_policy(
    context: &ExecutionContext,
    session: &AutomationSession,
    fallback: NavigationPolicy,
) -> NavigationPolicy {
    context
        .cache
        .read()
        .await
        .get(&policy_key(session))
        .and_then(|entry| entry.as_any().downcast_ref::<PolicyEntry>())
        .map(|entry| entry.0.clone())
        .unwrap_or(fallback)
}

/// Checks where a navigation ended (after redirects) and leaves the page when it is blocked.
#[cfg(feature = "execute")]
pub(crate) async fn verify_landing(
    driver: &thirtyfour::WebDriver,
    policy: &NavigationPolicy,
    requested: &str,
) -> flow_like_types::Result<String> {
    let landed = driver
        .current_url()
        .await
        .map_err(|error| flow_like_types::anyhow!("Failed to read the URL after navigating to '{requested}': {error}"))?
        .to_string();
    if let Err(error) = policy.check(&landed).await {
        let _ = driver.goto("about:blank").await;
        return Err(flow_like_types::anyhow!(
            "Navigation to '{requested}' ended on a blocked page and was left: {error}"
        ));
    }
    Ok(landed)
}

#[cfg(any(feature = "execute", test))]
fn validate_policy(policy: &NavigationPolicy) -> flow_like_types::Result<()> {
    for scheme in &policy.allowed_schemes {
        let scheme = scheme.trim().trim_end_matches(':');
        if scheme.is_empty()
            || !scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return Err(flow_like_types::anyhow!(
                "Invalid URL scheme '{scheme}' in the navigation policy"
            ));
        }
    }
    for pattern in policy.allowed_domains.iter().chain(&policy.blocked_domains) {
        if normalize_pattern(pattern).is_empty() || pattern.trim().contains(char::is_whitespace) {
            return Err(flow_like_types::anyhow!(
                "Invalid domain pattern '{pattern}' in the navigation policy"
            ));
        }
    }
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserSetNavigationPolicyNode {}

impl BrowserSetNavigationPolicyNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserSetNavigationPolicyNode {
    fn get_node(&self) -> Node {
        let mut node = super::manage::base_node(
            "browser_set_navigation_policy",
            "Set Navigation Policy",
            "Restricts where Go To and Execute Browser Action Plan may navigate in this session: URL schemes, domain allow and block lists, and private network addresses. The final URL after redirects is checked too; a blocked landing page is left for about:blank and the node fails. Requests the page makes on its own are not filtered.",
        );
        node.category = "Automation/Browser/Navigation".to_string();
        node.set_flowscript_name("browser", "setNavigationPolicy");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(7)
                .set_security(9)
                .set_performance(9)
                .set_governance(9)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "enabled",
            "Enabled",
            "Disable to remove the policy and restore the default rules (privileged schemes such as file: and javascript: stay blocked)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "allowed_schemes",
            "Allowed Schemes",
            "URL schemes that may be opened; empty allows all except file, javascript, chrome, edge, devtools and view-source",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!(["http", "https"])));
        node.add_input_pin(
            "allowed_domains",
            "Allowed Domains",
            "Host globs such as example.com or *.example.com; empty allows any host",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));
        node.add_input_pin(
            "blocked_domains",
            "Blocked Domains",
            "Host globs that are always blocked",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));
        node.add_input_pin(
            "block_private_networks",
            "Block Private Networks",
            "Block loopback, private, link-local and cloud metadata addresses (hosts are resolved; unresolvable hosts are blocked)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_output_pin(
            "policy",
            "Policy",
            "The policy now in effect",
            VariableType::Struct,
        )
        .set_schema::<NavigationPolicy>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let enabled: bool = context.evaluate_pin("enabled").await?;
        let policy = NavigationPolicy {
            allowed_schemes: context.evaluate_pin("allowed_schemes").await?,
            allowed_domains: context.evaluate_pin("allowed_domains").await?,
            blocked_domains: context.evaluate_pin("blocked_domains").await?,
            block_private_networks: context.evaluate_pin("block_private_networks").await?,
        };
        let effective = if enabled {
            validate_policy(&policy)?;
            context
                .cache
                .write()
                .await
                .insert(policy_key(&session), std::sync::Arc::new(PolicyEntry(policy.clone())));
            policy
        } else {
            context.cache.write().await.remove(&policy_key(&session));
            NavigationPolicy::goto_default()
        };
        context.set_pin_value("policy", json!(effective)).await?;
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

    fn check(policy: &NavigationPolicy, url: &str) -> bool {
        NavigationPolicy::parse(url)
            .and_then(|url| policy.check_static(&url))
            .is_ok()
    }

    #[test]
    fn default_goto_blocks_privileged_schemes_only() {
        let policy = NavigationPolicy::goto_default();
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "chrome://settings",
            "edge://settings",
            "view-source:https://example.com",
            "devtools://devtools/bundled/inspector.html",
        ] {
            assert!(!check(&policy, url), "{url} should be blocked");
        }
        for url in [
            "https://example.com",
            "http://localhost:3000",
            "about:blank",
            "data:text/html,hi",
        ] {
            assert!(check(&policy, url), "{url} should be allowed");
        }
        assert!(NavigationPolicy::parse("example.com").is_err());
    }

    #[test]
    fn plan_default_is_http_only() {
        let policy = NavigationPolicy::plan_default();
        assert!(check(&policy, "https://example.com/a"));
        assert!(!check(&policy, "about:blank"));
        assert!(!check(&policy, "data:text/html,x"));
        assert!(!check(&policy, "javascript:alert(1)"));
    }

    #[test]
    fn explicit_schemes_allow_privileged_ones() {
        let policy = NavigationPolicy {
            allowed_schemes: vec!["https".into(), "file:".into()],
            ..NavigationPolicy::default()
        };
        assert!(check(&policy, "file:///tmp/report.html"));
        assert!(!check(&policy, "http://example.com"));
    }

    #[test]
    fn domain_globs() {
        assert!(host_matches("*.example.com", "a.example.com"));
        assert!(host_matches("*.example.com", "a.b.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(!host_matches("*.example.com", "badexample.com"));
        assert!(host_matches("Example.COM.", "example.com"));
        assert!(host_matches("https://example.com:8443/path", "example.com"));
        assert!(host_matches("api-*.example.com", "api-eu.example.com"));
        assert!(host_matches("[::1]", "::1"));
        assert!(!host_matches("", "example.com"));
        let policy = NavigationPolicy {
            allowed_domains: vec!["example.com".into(), "*.example.com".into()],
            blocked_domains: vec!["admin.example.com".into()],
            ..NavigationPolicy::default()
        };
        assert!(check(&policy, "https://example.com"));
        assert!(check(&policy, "https://www.example.com./x"));
        assert!(!check(&policy, "https://admin.example.com"));
        assert!(!check(&policy, "https://example.org"));
        assert!(check(&policy, "about:blank"));
    }

    #[test]
    fn private_address_ranges() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.100.100.200",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "::1",
            "::",
            "fd00:ec2::254",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::a9fe:a9fe",
            "2002:0a00:0001::1",
            "::127.0.0.1",
        ] {
            assert!(is_private_ip(ip.parse().unwrap()), "{ip} should be private");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "100.63.255.255", "2606:4700:4700::1111", "::ffff:8.8.8.8"] {
            assert!(!is_private_ip(ip.parse().unwrap()), "{ip} should be public");
        }
    }

    #[test]
    fn private_networks_block_literals_and_local_names() {
        let policy = NavigationPolicy {
            block_private_networks: true,
            ..NavigationPolicy::default()
        };
        for url in [
            "http://169.254.169.254/latest/meta-data",
            "http://[fd00:ec2::254]/",
            "http://2130706433/",
            "http://0x7f.1/",
            "http://localhost:8080",
            "http://app.localhost",
            "http://metadata.google.internal/computeMetadata/v1",
            "http://[::ffff:10.0.0.1]/",
        ] {
            assert!(!check(&policy, url), "{url} should be blocked");
        }
        assert!(check(&policy, "https://93.184.216.34/"));
        assert!(check(&policy, "https://example.com/"));
    }

    #[test]
    fn policy_validation() {
        assert!(validate_policy(&NavigationPolicy::plan_default()).is_ok());
        let bad_scheme = NavigationPolicy {
            allowed_schemes: vec!["ht tp".into()],
            ..NavigationPolicy::default()
        };
        assert!(validate_policy(&bad_scheme).is_err());
        let bad_domain = NavigationPolicy {
            blocked_domains: vec!["  ".into()],
            ..NavigationPolicy::default()
        };
        assert!(validate_policy(&bad_domain).is_err());
    }
}
