use super::{
    list_issues::GitHubIssueUser,
    provider::{GITHUB_API_VERSION, GITHUB_PROVIDER_ID, GitHubProvider},
};
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubReleaseAsset {
    pub id: i64,
    pub name: String,
    pub label: Option<String>,
    pub content_type: String,
    pub size: i64,
    pub download_count: i64,
    pub browser_download_url: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubRelease {
    pub id: i64,
    pub tag_name: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub html_url: String,
    pub tarball_url: Option<String>,
    pub zipball_url: Option<String>,
    pub author: GitHubIssueUser,
    pub assets: Vec<GitHubReleaseAsset>,
    pub created_at: String,
    pub published_at: Option<String>,
}

fn parse_asset(asset: &Value) -> Option<GitHubReleaseAsset> {
    Some(GitHubReleaseAsset {
        id: asset["id"].as_i64()?,
        name: asset["name"].as_str()?.to_string(),
        label: asset["label"].as_str().map(String::from),
        content_type: asset["content_type"].as_str()?.to_string(),
        size: asset["size"].as_i64().unwrap_or(0),
        download_count: asset["download_count"].as_i64().unwrap_or(0),
        browser_download_url: asset["browser_download_url"].as_str()?.to_string(),
        created_at: asset["created_at"].as_str()?.to_string(),
        updated_at: asset["updated_at"].as_str()?.to_string(),
    })
}

fn filename_from_flow_path(path: &FlowPath, fallback: &str) -> String {
    Path::new(&path.path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

pub fn parse_release(release: &Value) -> Option<GitHubRelease> {
    let author = &release["author"];
    Some(GitHubRelease {
        id: release["id"].as_i64()?,
        tag_name: release["tag_name"].as_str()?.to_string(),
        name: release["name"].as_str().map(String::from),
        body: release["body"].as_str().map(String::from),
        draft: release["draft"].as_bool().unwrap_or(false),
        prerelease: release["prerelease"].as_bool().unwrap_or(false),
        html_url: release["html_url"].as_str()?.to_string(),
        tarball_url: release["tarball_url"].as_str().map(String::from),
        zipball_url: release["zipball_url"].as_str().map(String::from),
        author: GitHubIssueUser {
            id: author["id"].as_i64()?,
            login: author["login"].as_str()?.to_string(),
            avatar_url: author["avatar_url"].as_str()?.to_string(),
            html_url: author["html_url"].as_str()?.to_string(),
        },
        assets: release["assets"]
            .as_array()
            .map(|arr| arr.iter().filter_map(parse_asset).collect())
            .unwrap_or_default(),
        created_at: release["created_at"].as_str()?.to_string(),
        published_at: release["published_at"].as_str().map(String::from),
    })
}

// =============================================================================
// List Releases Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListGitHubReleasesNode {}

impl ListGitHubReleasesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGitHubReleasesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_list_releases",
            "List Releases",
            "List releases for a repository",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

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
            "per_page",
            "Per Page",
            "Results per page (max 100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));

        node.add_input_pin("page", "Page", "Page number", VariableType::Integer)
            .set_default_value(Some(json!(1)));

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
            "releases",
            "Releases",
            "Array of releases",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubRelease>();

        node.add_output_pin(
            "count",
            "Count",
            "Number of releases returned",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(9)
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
        let per_page: i64 = context.evaluate_pin("per_page").await.unwrap_or(30);
        let page: i64 = context.evaluate_pin("page").await.unwrap_or(1);

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/releases?per_page={}&page={}",
            owner,
            repo,
            per_page.clamp(1, 100),
            page.max(1)
        ));

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("GitHub API error {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let releases_json: Vec<Value> = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let releases: Vec<GitHubRelease> =
                    releases_json.iter().filter_map(parse_release).collect();

                let count = releases.len() as i64;

                context.log_message(&format!("Found {} releases", count), LogLevel::Info);
                context.set_pin_value("releases", json!(releases)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get Latest Release Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetLatestGitHubReleaseNode {}

impl GetLatestGitHubReleaseNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetLatestGitHubReleaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_get_latest_release",
            "Get Latest Release",
            "Get the latest published release (excludes drafts and prereleases)",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

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

        node.add_output_pin("release", "Release", "Latest release", VariableType::Struct)
            .set_schema::<GitHubRelease>();

        node.add_output_pin(
            "tag_name",
            "Tag Name",
            "Release tag name",
            VariableType::String,
        );
        node.add_output_pin("name", "Name", "Release name", VariableType::String);

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(9)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(9)
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

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!("/repos/{}/{}/releases/latest", owner, repo));

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("GitHub API error {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let release_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(release) = parse_release(&release_json) {
                    context
                        .set_pin_value("tag_name", json!(release.tag_name.clone()))
                        .await?;
                    context
                        .set_pin_value("name", json!(release.name.clone().unwrap_or_default()))
                        .await?;
                    context.set_pin_value("release", json!(release)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse release", LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get Release by Tag Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGitHubReleaseByTagNode {}

impl GetGitHubReleaseByTagNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGitHubReleaseByTagNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_get_release_by_tag",
            "Get Release by Tag",
            "Get a release by its tag name",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

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
            "tag",
            "Tag",
            "Tag name (e.g., v1.0.0)",
            VariableType::String,
        );

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
            "release",
            "Release",
            "Release details",
            VariableType::Struct,
        )
        .set_schema::<GitHubRelease>();

        node.add_output_pin("body", "Body", "Release notes", VariableType::String);

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(9)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(9)
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
        let tag: String = context.evaluate_pin("tag").await?;

        if owner.is_empty() || repo.is_empty() || tag.is_empty() {
            context.log_message("Owner, repository, and tag are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/releases/tags/{}",
            owner,
            repo,
            urlencoding::encode(&tag)
        ));

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("GitHub API error {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let release_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(release) = parse_release(&release_json) {
                    context
                        .set_pin_value("body", json!(release.body.clone().unwrap_or_default()))
                        .await?;
                    context.set_pin_value("release", json!(release)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse release", LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Create Release Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGitHubReleaseNode {}

impl CreateGitHubReleaseNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGitHubReleaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_create_release",
            "Create Release",
            "Create a new release",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

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
            "tag_name",
            "Tag Name",
            "Tag name for the release (e.g., v1.0.0)",
            VariableType::String,
        );

        node.add_input_pin(
            "name",
            "Name",
            "Release title (defaults to tag name if empty)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "body",
            "Body",
            "Release notes (Markdown supported)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "target_commitish",
            "Target",
            "Branch or SHA to tag (default: default branch)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "draft",
            "Draft",
            "Create as draft release",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "prerelease",
            "Prerelease",
            "Mark as prerelease",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "generate_release_notes",
            "Generate Notes",
            "Auto-generate release notes from commits",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "discussion_category_name",
            "Discussion Category",
            "Discussion category name to link a discussion to the release",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "make_latest",
            "Make Latest",
            "Controls whether this release is the latest release",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "".to_string(),
                    "true".to_string(),
                    "false".to_string(),
                    "legacy".to_string(),
                ])
                .build(),
        );

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
            "release",
            "Release",
            "Created release",
            VariableType::Struct,
        )
        .set_schema::<GitHubRelease>();

        node.add_output_pin(
            "release_id",
            "Release ID",
            "ID of the created release",
            VariableType::Integer,
        );
        node.add_output_pin(
            "html_url",
            "URL",
            "URL to the release",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
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
        let tag_name: String = context.evaluate_pin("tag_name").await?;
        let name: String = context.evaluate_pin("name").await.unwrap_or_default();
        let body: String = context.evaluate_pin("body").await.unwrap_or_default();
        let target_commitish: String = context
            .evaluate_pin("target_commitish")
            .await
            .unwrap_or_default();
        let draft: bool = context.evaluate_pin("draft").await.unwrap_or(false);
        let prerelease: bool = context.evaluate_pin("prerelease").await.unwrap_or(false);
        let generate_release_notes: bool = context
            .evaluate_pin("generate_release_notes")
            .await
            .unwrap_or(false);
        let discussion_category_name: String = context
            .evaluate_pin("discussion_category_name")
            .await
            .unwrap_or_default();
        let make_latest: String = context
            .evaluate_pin("make_latest")
            .await
            .unwrap_or_default();

        if owner.is_empty() || repo.is_empty() || tag_name.is_empty() {
            context.log_message(
                "Owner, repository, and tag name are required",
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!("/repos/{}/{}/releases", owner, repo));

        let mut request_body = json!({
            "tag_name": tag_name,
            "draft": draft,
            "prerelease": prerelease,
            "generate_release_notes": generate_release_notes
        });

        if !name.is_empty() {
            request_body["name"] = json!(name);
        }
        if !body.is_empty() {
            request_body["body"] = json!(body);
        }
        if !target_commitish.is_empty() {
            request_body["target_commitish"] = json!(target_commitish);
        }
        if !discussion_category_name.is_empty() {
            request_body["discussion_category_name"] = json!(discussion_category_name);
        }
        if !make_latest.is_empty() {
            request_body["make_latest"] = json!(make_latest);
        }

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "flow-like")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("GitHub API error {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let release_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(release) = parse_release(&release_json) {
                    context.log_message(
                        &format!("Created release: {}", release.tag_name),
                        LogLevel::Info,
                    );
                    context
                        .set_pin_value("release_id", json!(release.id))
                        .await?;
                    context
                        .set_pin_value("html_url", json!(release.html_url.clone()))
                        .await?;
                    context.set_pin_value("release", json!(release)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse created release", LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// List Release Assets Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListGitHubReleaseAssetsNode {}

impl ListGitHubReleaseAssetsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGitHubReleaseAssetsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_list_release_assets",
            "List Release Assets",
            "List assets attached to a release",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
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
            "release_id",
            "Release ID",
            "Release ID",
            VariableType::Integer,
        );
        node.add_input_pin(
            "per_page",
            "Per Page",
            "Results per page (max 100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));
        node.add_input_pin("page", "Page", "Page number", VariableType::Integer)
            .set_default_value(Some(json!(1)));

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
        node.add_output_pin("assets", "Assets", "Release assets", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<GitHubReleaseAsset>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin("count", "Count", "Number of assets", VariableType::Integer);

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
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
        let release_id: i64 = context.evaluate_pin("release_id").await?;
        let per_page: i64 = context.evaluate_pin("per_page").await.unwrap_or(30);
        let page: i64 = context.evaluate_pin("page").await.unwrap_or(1);

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/releases/{}/assets?per_page={}&page={}",
            owner,
            repo,
            release_id,
            per_page.clamp(1, 100),
            page.max(1)
        ));

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("GitHub API error {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let assets_json: Vec<Value> = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;
                let assets: Vec<GitHubReleaseAsset> =
                    assets_json.iter().filter_map(parse_asset).collect();
                let count = assets.len() as i64;

                context.set_pin_value("assets", json!(assets)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Upload Release Asset Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct UploadGitHubReleaseAssetNode {}

impl UploadGitHubReleaseAssetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UploadGitHubReleaseAssetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_upload_release_asset",
            "Upload Release Asset",
            "Upload a FlowPath file as a release asset",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
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
            "release_id",
            "Release ID",
            "Release ID",
            VariableType::Integer,
        );
        node.add_input_pin(
            "file",
            "File",
            "FlowPath file to upload",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "name",
            "Name",
            "Asset file name. Uses the FlowPath file name when empty",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin("label", "Label", "Asset label", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "content_type",
            "Content Type",
            "Asset MIME type",
            VariableType::String,
        )
        .set_default_value(Some(json!("application/octet-stream")));

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
        node.add_output_pin("asset", "Asset", "Uploaded asset", VariableType::Struct)
            .set_schema::<GitHubReleaseAsset>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "asset_id",
            "Asset ID",
            "Uploaded asset ID",
            VariableType::Integer,
        );
        node.add_output_pin(
            "download_url",
            "Download URL",
            "Browser download URL",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(5)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(7)
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
        let release_id: i64 = context.evaluate_pin("release_id").await?;
        let file: FlowPath = context.evaluate_pin("file").await?;
        let name: String = context.evaluate_pin("name").await.unwrap_or_default();
        let label: String = context.evaluate_pin("label").await.unwrap_or_default();
        let content_type: String = context
            .evaluate_pin("content_type")
            .await
            .unwrap_or_else(|_| "application/octet-stream".to_string());

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let bytes = file.get(context, false).await?;
        let asset_name = if name.is_empty() {
            filename_from_flow_path(&file, "release-asset.bin")
        } else {
            name
        };

        let mut url = provider.upload_api_url(&format!(
            "/repos/{}/{}/releases/{}/assets?name={}",
            owner,
            repo,
            release_id,
            urlencoding::encode(&asset_name)
        ));
        if !label.is_empty() {
            url.push_str(&format!("&label={}", urlencoding::encode(&label)));
        }

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("Content-Type", content_type)
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "flow-like")
            .body(bytes)
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("GitHub API error {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let asset_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;
                if let Some(asset) = parse_asset(&asset_json) {
                    context.set_pin_value("asset_id", json!(asset.id)).await?;
                    context
                        .set_pin_value("download_url", json!(asset.browser_download_url.clone()))
                        .await?;
                    context.set_pin_value("asset", json!(asset)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse uploaded asset", LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Download Release Asset Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DownloadGitHubReleaseAssetNode {}

impl DownloadGitHubReleaseAssetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DownloadGitHubReleaseAssetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_download_release_asset",
            "Download Release Asset",
            "Download a release asset into a FlowPath",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
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
            "asset_id",
            "Asset ID",
            "Release asset ID",
            VariableType::Integer,
        );
        node.add_input_pin(
            "output_path",
            "Output Path",
            "FlowPath to write the downloaded asset into",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

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
        node.add_output_pin("path", "Path", "Written file path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin("size", "Size", "File size in bytes", VariableType::Integer);

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(7)
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
        let asset_id: i64 = context.evaluate_pin("asset_id").await?;
        let output_path: FlowPath = context.evaluate_pin("output_path").await?;

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/releases/assets/{}",
            owner, repo, asset_id
        ));

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/octet-stream")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("GitHub API error {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to read response: {}", e))?;
                let size = bytes.len() as i64;
                output_path.put(context, bytes.to_vec(), false).await?;

                context.set_pin_value("path", json!(output_path)).await?;
                context.set_pin_value("size", json!(size)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
