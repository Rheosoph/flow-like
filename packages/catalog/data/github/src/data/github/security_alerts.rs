use super::provider::{GITHUB_API_VERSION, GITHUB_PROVIDER_ID, GitHubProvider};
use flow_like::flow::execution::egress::GuardedHttpClient;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json};
use serde::{Deserialize, Serialize};

const DEFAULT_MAX_ITEMS: i64 = 500;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubDependabotAlert {
    pub number: i64,
    pub state: String,
    pub severity: Option<String>,
    pub ghsa_id: Option<String>,
    pub cve_id: Option<String>,
    pub summary: Option<String>,
    pub cvss_score: Option<f64>,
    pub package_ecosystem: Option<String>,
    pub package_name: Option<String>,
    pub vulnerable_version_range: Option<String>,
    pub first_patched_version: Option<String>,
    pub manifest_path: Option<String>,
    pub scope: Option<String>,
    pub relationship: Option<String>,
    pub repository: Option<String>,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub fixed_at: Option<String>,
    pub dismissed_at: Option<String>,
    pub dismissed_reason: Option<String>,
    pub auto_dismissed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubCodeScanningAlert {
    pub number: i64,
    pub state: String,
    pub rule_id: Option<String>,
    pub rule_name: Option<String>,
    pub rule_description: Option<String>,
    pub severity: Option<String>,
    pub security_severity: Option<String>,
    pub tool_name: Option<String>,
    pub tool_version: Option<String>,
    pub path: Option<String>,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub message: Option<String>,
    pub ref_name: Option<String>,
    pub repository: Option<String>,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub fixed_at: Option<String>,
    pub dismissed_at: Option<String>,
    pub dismissed_reason: Option<String>,
}

/// A secret scanning alert without the secret value itself.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubSecretScanningAlert {
    pub number: i64,
    pub state: String,
    pub secret_type: String,
    pub secret_type_display_name: Option<String>,
    pub validity: Option<String>,
    pub resolution: Option<String>,
    pub resolved_at: Option<String>,
    pub push_protection_bypassed: Option<bool>,
    pub publicly_leaked: Option<bool>,
    pub multi_repo: Option<bool>,
    pub first_location_path: Option<String>,
    pub first_location_line: Option<i64>,
    pub has_more_locations: Option<bool>,
    pub repository: Option<String>,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: Option<String>,
}

fn opt_str(value: &Value) -> Option<String> {
    value.as_str().map(String::from)
}

fn repository_name(alert: &Value) -> Option<String> {
    opt_str(&alert["repository"]["full_name"])
}

pub(crate) fn parse_dependabot_alert(alert: &Value) -> Option<GitHubDependabotAlert> {
    let advisory = &alert["security_advisory"];
    let vulnerability = &alert["security_vulnerability"];
    let dependency = &alert["dependency"];
    let package = if dependency["package"].is_object() {
        &dependency["package"]
    } else {
        &vulnerability["package"]
    };
    Some(GitHubDependabotAlert {
        number: alert["number"].as_i64()?,
        state: alert["state"].as_str()?.to_string(),
        severity: opt_str(&vulnerability["severity"]).or_else(|| opt_str(&advisory["severity"])),
        ghsa_id: opt_str(&advisory["ghsa_id"]),
        cve_id: opt_str(&advisory["cve_id"]),
        summary: opt_str(&advisory["summary"]),
        cvss_score: advisory["cvss"]["score"].as_f64(),
        package_ecosystem: opt_str(&package["ecosystem"]),
        package_name: opt_str(&package["name"]),
        vulnerable_version_range: opt_str(&vulnerability["vulnerable_version_range"]),
        first_patched_version: opt_str(&vulnerability["first_patched_version"]["identifier"]),
        manifest_path: opt_str(&dependency["manifest_path"]),
        scope: opt_str(&dependency["scope"]),
        relationship: opt_str(&dependency["relationship"]),
        repository: repository_name(alert),
        html_url: alert["html_url"].as_str()?.to_string(),
        created_at: alert["created_at"].as_str()?.to_string(),
        updated_at: opt_str(&alert["updated_at"]),
        fixed_at: opt_str(&alert["fixed_at"]),
        dismissed_at: opt_str(&alert["dismissed_at"]),
        dismissed_reason: opt_str(&alert["dismissed_reason"]),
        auto_dismissed_at: opt_str(&alert["auto_dismissed_at"]),
    })
}

pub(crate) fn parse_code_scanning_alert(alert: &Value) -> Option<GitHubCodeScanningAlert> {
    let rule = &alert["rule"];
    let instance = &alert["most_recent_instance"];
    let location = &instance["location"];
    Some(GitHubCodeScanningAlert {
        number: alert["number"].as_i64()?,
        state: alert["state"].as_str()?.to_string(),
        rule_id: opt_str(&rule["id"]),
        rule_name: opt_str(&rule["name"]),
        rule_description: opt_str(&rule["description"]),
        severity: opt_str(&rule["severity"]),
        security_severity: opt_str(&rule["security_severity_level"]),
        tool_name: opt_str(&alert["tool"]["name"]),
        tool_version: opt_str(&alert["tool"]["version"]),
        path: opt_str(&location["path"]),
        start_line: location["start_line"].as_i64(),
        end_line: location["end_line"].as_i64(),
        message: opt_str(&instance["message"]["text"]),
        ref_name: opt_str(&instance["ref"]),
        repository: repository_name(alert),
        html_url: alert["html_url"].as_str()?.to_string(),
        created_at: alert["created_at"].as_str()?.to_string(),
        updated_at: opt_str(&alert["updated_at"]),
        fixed_at: opt_str(&alert["fixed_at"]),
        dismissed_at: opt_str(&alert["dismissed_at"]),
        dismissed_reason: opt_str(&alert["dismissed_reason"]),
    })
}

pub(crate) fn parse_secret_scanning_alert(alert: &Value) -> Option<GitHubSecretScanningAlert> {
    let location = &alert["first_location_detected"];
    Some(GitHubSecretScanningAlert {
        number: alert["number"].as_i64()?,
        state: alert["state"].as_str()?.to_string(),
        secret_type: alert["secret_type"].as_str()?.to_string(),
        secret_type_display_name: opt_str(&alert["secret_type_display_name"]),
        validity: opt_str(&alert["validity"]),
        resolution: opt_str(&alert["resolution"]),
        resolved_at: opt_str(&alert["resolved_at"]),
        push_protection_bypassed: alert["push_protection_bypassed"].as_bool(),
        publicly_leaked: alert["publicly_leaked"].as_bool(),
        multi_repo: alert["multi_repo"].as_bool(),
        first_location_path: opt_str(&location["path"]),
        first_location_line: location["start_line"].as_i64(),
        has_more_locations: alert["has_more_locations"].as_bool(),
        repository: repository_name(alert),
        html_url: alert["html_url"].as_str()?.to_string(),
        created_at: alert["created_at"].as_str()?.to_string(),
        updated_at: opt_str(&alert["updated_at"]),
    })
}

/// The `rel="next"` target of a GitHub `Link` header.
pub(crate) fn next_page_url(link_header: &str) -> Option<String> {
    link_header.split(',').find_map(|part| {
        let mut segments = part.split(';');
        let url = segments
            .next()?
            .trim()
            .strip_prefix('<')?
            .strip_suffix('>')?;
        segments
            .any(|segment| segment.trim() == "rel=\"next\"")
            .then(|| url.to_string())
    })
}

/// Endpoint for a repository, or the organization-wide endpoint when `repo` is empty.
pub(crate) fn alerts_path(owner: &str, repo: &str, kind: &str, query: &[(&str, String)]) -> String {
    let owner = urlencoding::encode(owner.trim());
    let repo = repo.trim();
    let base = if repo.is_empty() {
        format!("/orgs/{owner}/{kind}/alerts")
    } else {
        format!("/repos/{owner}/{}/{kind}/alerts", urlencoding::encode(repo))
    };
    let mut params = vec!["per_page=100".to_string()];
    params.extend(
        query
            .iter()
            .filter(|(_, value)| !value.trim().is_empty())
            .map(|(key, value)| format!("{key}={}", urlencoding::encode(value.trim()))),
    );
    format!("{base}?{}", params.join("&"))
}

pub(crate) struct Page {
    pub items: Vec<Value>,
    pub next: Option<String>,
}

/// Appends a page and reports whether collection should stop, either because the cap was reached
/// or because there is no further page. The second value is `true` when items were cut off.
pub(crate) fn collect_page(
    collected: &mut Vec<Value>,
    page: Page,
    max_items: usize,
) -> (bool, bool) {
    let available = page.items.len();
    let room = max_items.saturating_sub(collected.len());
    collected.extend(page.items.into_iter().take(room));
    let cut = available > room;
    let truncated = cut || (collected.len() >= max_items && page.next.is_some());
    let done = truncated || page.next.is_none() || collected.len() >= max_items;
    (done, truncated)
}

async fn fetch_page(
    client: &GuardedHttpClient,
    provider: &GitHubProvider,
    url: &str,
) -> flow_like_types::Result<Page> {
    let response = client
        .get(url)?
        .header("Authorization", provider.auth_header())
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .header("User-Agent", "flow-like")
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(flow_like_types::anyhow!(
            "GitHub API error {} for {}: {}",
            status,
            url,
            text
        ));
    }
    let next = response
        .headers()
        .get("link")
        .and_then(|value| value.to_str().ok())
        .and_then(next_page_url);
    let items: Vec<Value> = response
        .json()
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to parse alerts from {}: {}", url, e))?;
    Ok(Page { items, next })
}

/// Follows `Link` pagination until `max_items` alerts are collected or no page is left.
pub(crate) async fn fetch_all_pages(
    client: &GuardedHttpClient,
    provider: &GitHubProvider,
    first_url: String,
    max_items: usize,
) -> flow_like_types::Result<(Vec<Value>, bool)> {
    let mut collected = Vec::new();
    let mut url = first_url;
    loop {
        let page = fetch_page(client, provider, &url).await?;
        let next = page.next.clone();
        let (done, truncated) = collect_page(&mut collected, page, max_items);
        if done {
            return Ok((collected, truncated));
        }
        match next {
            Some(next) => url = next,
            None => return Ok((collected, truncated)),
        }
    }
}

struct AlertNodeSpec {
    id: &'static str,
    friendly_name: &'static str,
    description: &'static str,
    flowscript: &'static str,
    states: &'static [&'static str],
    severities: &'static [&'static str],
    output_description: &'static str,
}

fn alert_node<T: JsonSchema + Serialize>(spec: &AlertNodeSpec) -> Node {
    let mut node = Node::new(spec.id, spec.friendly_name, spec.description, "Data/GitHub");
    node.set_flowscript_name("github", spec.flowscript);
    node.add_icon("/flow/icons/github.svg");

    node.add_input_pin(
        "exec_in",
        "Input",
        "Fetch the alerts",
        VariableType::Execution,
    );

    node.add_input_pin(
        "provider",
        "Provider",
        "GitHub provider",
        VariableType::Struct,
    )
    .set_schema::<GitHubProvider>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());

    node.add_input_pin(
        "owner",
        "Owner",
        "Repository owner or organization",
        VariableType::String,
    );

    node.add_input_pin(
        "repo",
        "Repository",
        "Repository name. Leave empty to list the alerts of the whole organization",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));

    let mut states = vec![String::new()];
    states.extend(spec.states.iter().map(|s| s.to_string()));
    node.add_input_pin(
        "state",
        "State",
        "Only alerts in this state. Empty returns all states",
        VariableType::String,
    )
    .set_default_value(Some(json!("open")))
    .set_options(PinOptions::new().set_valid_values(states).build());

    if !spec.severities.is_empty() {
        let mut severities = vec![String::new()];
        severities.extend(spec.severities.iter().map(|s| s.to_string()));
        node.add_input_pin(
            "severity",
            "Severity",
            "Only alerts with this severity. Empty returns all severities",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(PinOptions::new().set_valid_values(severities).build());
    }

    node.add_input_pin(
        "max_items",
        "Max Items",
        "Stop after this many alerts",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(DEFAULT_MAX_ITEMS)));

    node.add_output_pin(
        "exec_out",
        "Success",
        "Triggered on success",
        VariableType::Execution,
    );
    node.add_output_pin(
        "error",
        "Error",
        "Triggered on error",
        VariableType::Execution,
    );

    node.add_output_pin(
        "alerts",
        "Alerts",
        spec.output_description,
        VariableType::Struct,
    )
    .set_value_type(ValueType::Array)
    .set_schema::<T>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());

    node.add_output_pin(
        "count",
        "Count",
        "Number of alerts returned",
        VariableType::Integer,
    );

    node.add_output_pin(
        "truncated",
        "Truncated",
        "True when more alerts exist than Max Items allowed",
        VariableType::Boolean,
    );

    node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo", "security_events"]);
    node.set_scores(
        NodeScores::new()
            .set_privacy(5)
            .set_security(8)
            .set_performance(7)
            .set_governance(8)
            .set_reliability(8)
            .set_cost(8)
            .build(),
    );

    node
}

async fn run_alert_node<T: Serialize>(
    context: &mut ExecutionContext,
    kind: &str,
    extra_query: &[(&str, &str)],
    has_severity: bool,
    parse: fn(&Value) -> Option<T>,
) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    context.deactivate_exec_pin("error").await?;

    let provider: GitHubProvider = context.evaluate_pin("provider").await?;
    let owner: String = context.evaluate_pin("owner").await?;
    let repo: String = context.evaluate_pin("repo").await.unwrap_or_default();
    let state: String = context.evaluate_pin("state").await.unwrap_or_default();
    let severity: String = if has_severity {
        context.evaluate_pin("severity").await.unwrap_or_default()
    } else {
        String::new()
    };
    let max_items: i64 = context
        .evaluate_pin("max_items")
        .await
        .unwrap_or(DEFAULT_MAX_ITEMS);

    if owner.trim().is_empty() {
        return Err(flow_like_types::anyhow!("Owner is required"));
    }

    let mut query = vec![("state", state), ("severity", severity)];
    query.extend(extra_query.iter().map(|(k, v)| (*k, v.to_string())));
    let url = provider.api_url(&alerts_path(&owner, &repo, kind, &query));
    let client = GuardedHttpClient::new(context.execution_environment())?;

    match fetch_all_pages(&client, &provider, url, max_items.max(1) as usize).await {
        Ok((raw, truncated)) => {
            let alerts: Vec<T> = raw.iter().filter_map(parse).collect();
            let skipped = raw.len() - alerts.len();
            if skipped > 0 {
                context.log_message(
                    &format!(
                        "Skipped {} {} alert(s) with an unexpected shape",
                        skipped, kind
                    ),
                    LogLevel::Warn,
                );
            }
            context.set_pin_value("count", json!(alerts.len())).await?;
            context.set_pin_value("alerts", json!(alerts)).await?;
            context.set_pin_value("truncated", json!(truncated)).await?;
            context.activate_exec_pin("exec_out").await?;
        }
        Err(e) => {
            context.log_message(&format!("{}", e), LogLevel::Error);
            context.activate_exec_pin("error").await?;
        }
    }
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct ListDependabotAlertsNode {}

impl ListDependabotAlertsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListDependabotAlertsNode {
    fn get_node(&self) -> Node {
        alert_node::<GitHubDependabotAlert>(&AlertNodeSpec {
            id: "data_github_list_dependabot_alerts",
            friendly_name: "List Dependabot Alerts",
            description: "List Dependabot alerts of a repository or organization: vulnerable dependencies with advisory (GHSA/CVE), affected package, vulnerable range and first patched version",
            flowscript: "listDependabotAlerts",
            states: &["open", "fixed", "dismissed", "auto_dismissed"],
            severities: &["critical", "high", "medium", "low"],
            output_description: "Dependabot alerts",
        })
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_alert_node(context, "dependabot", &[], true, parse_dependabot_alert).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ListCodeScanningAlertsNode {}

impl ListCodeScanningAlertsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListCodeScanningAlertsNode {
    fn get_node(&self) -> Node {
        alert_node::<GitHubCodeScanningAlert>(&AlertNodeSpec {
            id: "data_github_list_code_scanning_alerts",
            friendly_name: "List Code Scanning Alerts",
            description: "List code scanning alerts (e.g. CodeQL) of a repository or organization with rule, tool, severity and the location of the most recent instance",
            flowscript: "listCodeScanningAlerts",
            states: &["open", "closed", "dismissed", "fixed"],
            severities: &[
                "critical", "high", "medium", "low", "warning", "note", "error",
            ],
            output_description: "Code scanning alerts",
        })
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_alert_node(
            context,
            "code-scanning",
            &[],
            true,
            parse_code_scanning_alert,
        )
        .await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ListSecretScanningAlertsNode {}

impl ListSecretScanningAlertsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListSecretScanningAlertsNode {
    fn get_node(&self) -> Node {
        alert_node::<GitHubSecretScanningAlert>(&AlertNodeSpec {
            id: "data_github_list_secret_scanning_alerts",
            friendly_name: "List Secret Scanning Alerts",
            description: "List secret scanning alerts of a repository or organization. The secret value is never requested or returned, only its type, validity, state and first location",
            flowscript: "listSecretScanningAlerts",
            states: &["open", "resolved"],
            severities: &[],
            output_description: "Secret scanning alerts without secret values",
        })
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_alert_node(
            context,
            "secret-scanning",
            &[("hide_secret", "true")],
            false,
            parse_secret_scanning_alert,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_page_is_taken_from_link_header() {
        let header = r#"<https://api.github.com/repositories/1/dependabot/alerts?per_page=100&after=Y3Vy>; rel="next", <https://api.github.com/repositories/1/dependabot/alerts?per_page=100&before=X>; rel="prev""#;
        assert_eq!(
            next_page_url(header).as_deref(),
            Some("https://api.github.com/repositories/1/dependabot/alerts?per_page=100&after=Y3Vy")
        );
        assert_eq!(next_page_url(r#"<https://x/a?page=1>; rel="first""#), None);
        assert_eq!(next_page_url(""), None);
    }

    #[test]
    fn alerts_path_switches_to_org_and_skips_empty_filters() {
        assert_eq!(
            alerts_path(
                "acme",
                "api",
                "dependabot",
                &[("state", "open".into()), ("severity", "".into())]
            ),
            "/repos/acme/api/dependabot/alerts?per_page=100&state=open"
        );
        assert_eq!(
            alerts_path(
                "acme",
                " ",
                "secret-scanning",
                &[("hide_secret", "true".into())]
            ),
            "/orgs/acme/secret-scanning/alerts?per_page=100&hide_secret=true"
        );
    }

    fn page(n: usize, next: bool) -> Page {
        Page {
            items: (0..n).map(|i| json!(i)).collect(),
            next: next.then(|| "https://next".to_string()),
        }
    }

    #[test]
    fn pagination_stops_at_cap_and_reports_truncation() {
        let mut collected = Vec::new();
        assert_eq!(
            collect_page(&mut collected, page(100, true), 150),
            (false, false)
        );
        assert_eq!(
            collect_page(&mut collected, page(100, true), 150),
            (true, true)
        );
        assert_eq!(collected.len(), 150);

        let mut collected = Vec::new();
        assert_eq!(
            collect_page(&mut collected, page(100, true), 100),
            (true, true)
        );

        let mut collected = Vec::new();
        assert_eq!(
            collect_page(&mut collected, page(100, false), 100),
            (true, false)
        );

        let mut collected = Vec::new();
        assert_eq!(
            collect_page(&mut collected, page(20, false), 500),
            (true, false)
        );
        assert_eq!(collected.len(), 20);
    }

    #[test]
    fn dependabot_alert_is_flattened() {
        let raw = json!({
            "number": 7,
            "state": "open",
            "dependency": {
                "package": {"ecosystem": "npm", "name": "lodash"},
                "manifest_path": "web/package-lock.json",
                "scope": "runtime",
                "relationship": "transitive"
            },
            "security_advisory": {
                "ghsa_id": "GHSA-jf85-cpcp-j695",
                "cve_id": "CVE-2019-10744",
                "summary": "Prototype Pollution in lodash",
                "severity": "critical",
                "cvss": {"score": 9.1}
            },
            "security_vulnerability": {
                "package": {"ecosystem": "npm", "name": "lodash"},
                "severity": "critical",
                "vulnerable_version_range": "< 4.17.12",
                "first_patched_version": {"identifier": "4.17.12"}
            },
            "html_url": "https://github.com/acme/api/security/dependabot/7",
            "created_at": "2026-01-01T00:00:00Z",
            "fixed_at": null,
            "repository": {"full_name": "acme/api"}
        });
        let alert = parse_dependabot_alert(&raw).unwrap();
        assert_eq!(alert.package_name.as_deref(), Some("lodash"));
        assert_eq!(alert.cve_id.as_deref(), Some("CVE-2019-10744"));
        assert_eq!(alert.vulnerable_version_range.as_deref(), Some("< 4.17.12"));
        assert_eq!(alert.first_patched_version.as_deref(), Some("4.17.12"));
        assert_eq!(
            alert.manifest_path.as_deref(),
            Some("web/package-lock.json")
        );
        assert_eq!(alert.relationship.as_deref(), Some("transitive"));
        assert_eq!(alert.cvss_score, Some(9.1));
        assert_eq!(alert.repository.as_deref(), Some("acme/api"));
        assert_eq!(alert.fixed_at, None);
    }

    #[test]
    fn code_scanning_alert_keeps_rule_tool_and_location() {
        let raw = json!({
            "number": 3,
            "state": "open",
            "rule": {"id": "js/sql-injection", "name": "Database query built from user-controlled sources", "severity": "error", "security_severity_level": "high"},
            "tool": {"name": "CodeQL", "version": "2.20.0"},
            "most_recent_instance": {
                "ref": "refs/heads/main",
                "message": {"text": "This query depends on a user-provided value."},
                "location": {"path": "src/db.js", "start_line": 42, "end_line": 44}
            },
            "html_url": "https://github.com/acme/api/security/code-scanning/3",
            "created_at": "2026-01-01T00:00:00Z"
        });
        let alert = parse_code_scanning_alert(&raw).unwrap();
        assert_eq!(alert.rule_id.as_deref(), Some("js/sql-injection"));
        assert_eq!(alert.security_severity.as_deref(), Some("high"));
        assert_eq!(alert.tool_name.as_deref(), Some("CodeQL"));
        assert_eq!(alert.path.as_deref(), Some("src/db.js"));
        assert_eq!(alert.start_line, Some(42));
        assert_eq!(alert.ref_name.as_deref(), Some("refs/heads/main"));
    }

    #[test]
    fn secret_scanning_alert_never_carries_the_secret() {
        let raw = json!({
            "number": 1,
            "state": "open",
            "secret_type": "aws_access_key_id",
            "secret_type_display_name": "AWS Access Key ID",
            "secret": "AKIAIOSFODNN7EXAMPLE",
            "validity": "active",
            "publicly_leaked": false,
            "first_location_detected": {"path": "config/dev.env", "start_line": 3},
            "has_more_locations": true,
            "html_url": "https://github.com/acme/api/security/secret-scanning/1",
            "created_at": "2026-01-01T00:00:00Z"
        });
        let alert = parse_secret_scanning_alert(&raw).unwrap();
        assert_eq!(alert.first_location_path.as_deref(), Some("config/dev.env"));
        assert_eq!(alert.first_location_line, Some(3));
        assert_eq!(alert.validity.as_deref(), Some("active"));
        let serialized = json!(alert).to_string();
        assert!(!serialized.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn alerts_with_missing_required_fields_are_skipped() {
        assert!(parse_dependabot_alert(&json!({"number": 1})).is_none());
        assert!(parse_code_scanning_alert(&json!({"state": "open"})).is_none());
        assert!(parse_secret_scanning_alert(&json!({"number": 1, "state": "open"})).is_none());
    }

    #[test]
    fn secret_scanning_node_has_no_severity_filter() {
        let node = ListSecretScanningAlertsNode::new().get_node();
        assert!(node.get_pin_by_name("severity").is_none());
        let node = ListDependabotAlertsNode::new().get_node();
        assert!(node.get_pin_by_name("severity").is_some());
    }
}
