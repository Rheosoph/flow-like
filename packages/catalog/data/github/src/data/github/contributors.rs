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
use std::{future::Future, time::Duration};

const SECONDS_PER_WEEK: i64 = 7 * 24 * 60 * 60;
const MAX_BACKOFF_FACTOR: u32 = 8;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct GitHubContributorWeek {
    /// Start of the week as Unix timestamp (seconds)
    pub week_start: i64,
    pub additions: i64,
    pub deletions: i64,
    pub commits: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct GitHubContributorStats {
    /// Login of the author; empty for commits whose author has no GitHub account
    pub login: Option<String>,
    pub author_type: Option<String>,
    pub total_commits: i64,
    pub recent_commits: i64,
    pub recent_additions: i64,
    pub recent_deletions: i64,
    pub weeks: Vec<GitHubContributorWeek>,
}

pub(crate) fn parse_contributor(
    entry: &Value,
    recent_since: i64,
) -> Option<GitHubContributorStats> {
    let weeks: Vec<GitHubContributorWeek> = entry["weeks"]
        .as_array()
        .map(|weeks| {
            weeks
                .iter()
                .filter_map(|week| {
                    Some(GitHubContributorWeek {
                        week_start: week["w"].as_i64()?,
                        additions: week["a"].as_i64().unwrap_or(0),
                        deletions: week["d"].as_i64().unwrap_or(0),
                        commits: week["c"].as_i64().unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let recent = weeks.iter().filter(|week| week.week_start >= recent_since);
    let (recent_commits, recent_additions, recent_deletions) = recent
        .fold((0, 0, 0), |(c, a, d), week| {
            (c + week.commits, a + week.additions, d + week.deletions)
        });
    Some(GitHubContributorStats {
        login: entry["author"]["login"].as_str().map(String::from),
        author_type: entry["author"]["type"].as_str().map(String::from),
        total_commits: entry["total"].as_i64()?,
        recent_commits,
        recent_additions,
        recent_deletions,
        weeks,
    })
}

/// Contributors with the most recent commits first, ties broken by total commits.
pub(crate) fn parse_contributors(body: &Value, recent_since: i64) -> Vec<GitHubContributorStats> {
    let mut contributors: Vec<GitHubContributorStats> = body
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| parse_contributor(entry, recent_since))
                .collect()
        })
        .unwrap_or_default();
    contributors.sort_by(|a, b| {
        b.recent_commits
            .cmp(&a.recent_commits)
            .then(b.total_commits.cmp(&a.total_commits))
    });
    contributors
}

pub(crate) fn backoff_delay(attempt: u32, base: Duration) -> Duration {
    base * 2u32.saturating_pow(attempt).min(MAX_BACKOFF_FACTOR)
}

/// GitHub answers `202 Accepted` while it computes repository statistics. Polls with
/// exponential backoff and returns `None` when the statistics are still not ready.
pub(crate) async fn poll_statistics<F, Fut>(
    max_attempts: u32,
    base_delay: Duration,
    mut fetch: F,
) -> flow_like_types::Result<Option<Value>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = flow_like_types::Result<(u16, Value)>>,
{
    let attempts = max_attempts.max(1);
    for attempt in 0..attempts {
        let (status, body) = fetch().await?;
        match status {
            200 => return Ok(Some(body)),
            204 => return Ok(Some(Value::Array(Vec::new()))),
            202 => {
                if attempt + 1 < attempts {
                    flow_like_types::tokio::time::sleep(backoff_delay(attempt, base_delay)).await;
                }
            }
            other => {
                return Err(flow_like_types::anyhow!(
                    "GitHub API error {} while reading contributor statistics: {}",
                    other,
                    body
                ));
            }
        }
    }
    Ok(None)
}

#[crate::register_node]
#[derive(Default)]
pub struct GetContributorStatsNode {}

impl GetContributorStatsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetContributorStatsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_get_contributor_stats",
            "Get Contributor Stats",
            "Weekly commit, addition and deletion statistics per contributor (top 100 by commits). GitHub computes these on demand, so the node retries while the statistics are being prepared. Repositories with 10,000 or more commits report zero additions and deletions.",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "getContributorStats");
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Fetch contributor statistics",
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

        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin(
            "repo",
            "Repository",
            "Repository name",
            VariableType::String,
        );

        node.add_input_pin(
            "recent_weeks",
            "Recent Weeks",
            "Window for the recent commit, addition and deletion totals",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(52)));

        node.add_input_pin(
            "max_attempts",
            "Max Attempts",
            "How often to ask while GitHub is still computing the statistics",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(6)));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on success",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Triggered on API errors or when the statistics are still not ready",
            VariableType::Execution,
        );

        node.add_output_pin(
            "contributors",
            "Contributors",
            "Contributors sorted by recent commits",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubContributorStats>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "recent_total",
            "Recent Commits",
            "Commits by all listed contributors within the recent window",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await?;
        let repo: String = context.evaluate_pin("repo").await?;
        let recent_weeks: i64 = context.evaluate_pin("recent_weeks").await.unwrap_or(52);
        let max_attempts: i64 = context.evaluate_pin("max_attempts").await.unwrap_or(6);

        let url = provider.api_url(&format!(
            "/repos/{}/{}/stats/contributors",
            urlencoding::encode(owner.trim()),
            urlencoding::encode(repo.trim())
        ));
        let client = GuardedHttpClient::new(context.execution_environment())?;
        let (client, url, provider_ref) = (&client, &url, &provider);
        let fetch = move || async move {
            let response = client
                .get(url)?
                .header("Authorization", provider_ref.auth_header())
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
                .header("User-Agent", "flow-like")
                .send()
                .await?;
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            let body =
                flow_like_types::json::from_str::<Value>(&text).unwrap_or(Value::String(text));
            Ok::<_, flow_like_types::Error>((status, body))
        };

        let result = poll_statistics(
            max_attempts.clamp(1, 20) as u32,
            Duration::from_secs(1),
            fetch,
        )
        .await;

        match result {
            Ok(Some(body)) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let recent_since = now - recent_weeks.max(0) * SECONDS_PER_WEEK;
                let contributors = parse_contributors(&body, recent_since);
                let recent_total: i64 = contributors.iter().map(|c| c.recent_commits).sum();
                context
                    .set_pin_value("contributors", json!(contributors))
                    .await?;
                context
                    .set_pin_value("recent_total", json!(recent_total))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(None) => {
                context.log_message(
                    &format!(
                        "GitHub is still computing contributor statistics for {}/{}; try again later",
                        owner, repo
                    ),
                    LogLevel::Warn,
                );
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context.log_message(&format!("{}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn runtime() -> flow_like_types::tokio::runtime::Runtime {
        flow_like_types::tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    type ScriptedResponse = std::future::Ready<flow_like_types::Result<(u16, Value)>>;

    fn scripted(
        responses: Vec<(u16, Value)>,
    ) -> (Arc<Mutex<usize>>, impl FnMut() -> ScriptedResponse) {
        let calls = Arc::new(Mutex::new(0usize));
        let counter = calls.clone();
        let fetch = move || {
            let mut n = counter.lock().unwrap();
            let response = responses[(*n).min(responses.len() - 1)].clone();
            *n += 1;
            std::future::ready(Ok(response))
        };
        (calls, fetch)
    }

    #[test]
    fn polling_retries_while_github_computes() {
        let (calls, fetch) = scripted(vec![
            (202, json!({})),
            (202, json!({})),
            (200, json!([{"total": 1, "weeks": []}])),
        ]);
        let body = runtime()
            .block_on(poll_statistics(5, Duration::ZERO, fetch))
            .unwrap();
        assert!(body.unwrap().is_array());
        assert_eq!(*calls.lock().unwrap(), 3);
    }

    #[test]
    fn polling_gives_up_after_max_attempts() {
        let (calls, fetch) = scripted(vec![(202, json!({}))]);
        let body = runtime()
            .block_on(poll_statistics(3, Duration::ZERO, fetch))
            .unwrap();
        assert!(body.is_none());
        assert_eq!(*calls.lock().unwrap(), 3);
    }

    #[test]
    fn polling_surfaces_api_errors_and_empty_repositories() {
        let (_, fetch) = scripted(vec![(404, json!({"message": "Not Found"}))]);
        let error = runtime()
            .block_on(poll_statistics(3, Duration::ZERO, fetch))
            .unwrap_err();
        assert!(error.to_string().contains("404"));

        let (_, fetch) = scripted(vec![(204, Value::Null)]);
        let body = runtime()
            .block_on(poll_statistics(3, Duration::ZERO, fetch))
            .unwrap();
        assert_eq!(body, Some(json!([])));
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        let base = Duration::from_secs(1);
        assert_eq!(backoff_delay(0, base), Duration::from_secs(1));
        assert_eq!(backoff_delay(2, base), Duration::from_secs(4));
        assert_eq!(backoff_delay(10, base), Duration::from_secs(8));
    }

    #[test]
    fn recent_window_and_sorting() {
        let body = json!([
            {
                "author": {"login": "old-timer", "type": "User"},
                "total": 500,
                "weeks": [
                    {"w": 1_000, "a": 10, "d": 1, "c": 400},
                    {"w": 9_000, "a": 5, "d": 2, "c": 1}
                ]
            },
            {
                "author": {"login": "active", "type": "User"},
                "total": 40,
                "weeks": [
                    {"w": 8_000, "a": 100, "d": 50, "c": 30},
                    {"w": 9_000, "a": 20, "d": 10, "c": 10}
                ]
            },
            {"author": null, "total": 3, "weeks": [{"w": 9_000, "a": 0, "d": 0, "c": 3}]}
        ]);
        let contributors = parse_contributors(&body, 8_000);
        let logins: Vec<_> = contributors.iter().map(|c| c.login.clone()).collect();
        assert_eq!(
            logins,
            vec![Some("active".into()), None, Some("old-timer".into())]
        );
        assert_eq!(contributors[0].recent_commits, 40);
        assert_eq!(contributors[0].recent_additions, 120);
        assert_eq!(contributors[0].recent_deletions, 60);
        assert_eq!(contributors[2].recent_commits, 1);
        assert_eq!(contributors[2].total_commits, 500);
        assert_eq!(contributors[2].weeks.len(), 2);
    }
}
