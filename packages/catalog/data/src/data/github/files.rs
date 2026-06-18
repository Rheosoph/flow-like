use super::provider::{GITHUB_API_VERSION, GITHUB_PROVIDER_ID, GitHubProvider};
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubFileContent {
    pub name: String,
    pub path: String,
    pub sha: String,
    pub size: i64,
    pub content: Option<String>,
    pub encoding: Option<String>,
    pub html_url: String,
    pub download_url: Option<String>,
    pub file_type: String,
}

fn parse_file_content(file: &Value) -> Option<GitHubFileContent> {
    Some(GitHubFileContent {
        name: file["name"].as_str()?.to_string(),
        path: file["path"].as_str()?.to_string(),
        sha: file["sha"].as_str()?.to_string(),
        size: file["size"].as_i64().unwrap_or(0),
        content: file["content"].as_str().map(String::from),
        encoding: file["encoding"].as_str().map(String::from),
        html_url: file["html_url"].as_str()?.to_string(),
        download_url: file["download_url"].as_str().map(String::from),
        file_type: file["type"].as_str()?.to_string(),
    })
}

// =============================================================================
// Get File Contents Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGitHubFileContentsNode {}

impl GetGitHubFileContentsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGitHubFileContentsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_get_file_contents",
            "Get File Contents",
            "Get the contents of a file from a repository",
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
            "path",
            "Path",
            "Path to file in the repository",
            VariableType::String,
        );

        node.add_input_pin(
            "ref",
            "Ref",
            "Branch, tag, or commit SHA (default: default branch)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

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
            "file_info",
            "File Info",
            "File metadata",
            VariableType::Struct,
        )
        .set_schema::<GitHubFileContent>();

        node.add_output_pin(
            "content",
            "Content",
            "Decoded UTF-8 file content",
            VariableType::String,
        );
        node.add_output_pin(
            "base64_content",
            "Base64 Content",
            "Exact file content returned by GitHub, without line breaks",
            VariableType::String,
        );
        node.add_output_pin(
            "sha",
            "SHA",
            "File SHA (needed for updates)",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
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
        let path: String = context.evaluate_pin("path").await?;
        let ref_param: String = context.evaluate_pin("ref").await.unwrap_or_default();

        if owner.is_empty() || repo.is_empty() || path.is_empty() {
            context.log_message("Owner, repository, and path are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let mut url = format!(
            "/repos/{}/{}/contents/{}",
            owner,
            repo,
            urlencoding::encode(&path)
        );

        if !ref_param.is_empty() {
            url.push_str(&format!("?ref={}", urlencoding::encode(&ref_param)));
        }

        let full_url = provider.api_url(&url);

        let client = reqwest::Client::new();
        let response = client
            .get(&full_url)
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

                let file_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(file_info) = parse_file_content(&file_json) {
                    let base64_content = file_info
                        .content
                        .as_ref()
                        .map(|content| content.replace('\n', ""))
                        .unwrap_or_default();

                    let decoded_content = if base64_content.is_empty() {
                        String::new()
                    } else {
                        match flow_like_types::base64::Engine::decode(
                            &flow_like_types::base64::prelude::BASE64_STANDARD,
                            &base64_content,
                        ) {
                            Ok(bytes) => String::from_utf8(bytes).unwrap_or_default(),
                            Err(_) => String::new(),
                        }
                    };

                    context
                        .set_pin_value("content", json!(decoded_content))
                        .await?;
                    context
                        .set_pin_value("base64_content", json!(base64_content))
                        .await?;
                    context
                        .set_pin_value("sha", json!(file_info.sha.clone()))
                        .await?;
                    context.set_pin_value("file_info", json!(file_info)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse file content", LogLevel::Error);
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
// Create or Update File Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateOrUpdateGitHubFileNode {}

impl CreateOrUpdateGitHubFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateOrUpdateGitHubFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_create_or_update_file",
            "Create/Update File",
            "Create or update a file in a repository",
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
            "path",
            "Path",
            "Path to file in the repository",
            VariableType::String,
        );
        node.add_input_pin(
            "source_file",
            "Source File",
            "FlowPath file to upload. When connected, this is used instead of Content",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "content",
            "Content",
            "Text content used when Source File is not connected",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "message",
            "Commit Message",
            "Commit message",
            VariableType::String,
        );

        node.add_input_pin(
            "sha",
            "SHA",
            "SHA of the file being replaced (required for updates, get from 'Get File Contents')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "branch",
            "Branch",
            "Branch to commit to (default: default branch)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "committer_name",
            "Committer Name",
            "Name of the committer (default: authenticated user)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "committer_email",
            "Committer Email",
            "Email of the committer (default: authenticated user's email)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "author_name",
            "Author Name",
            "Name of the author (default: authenticated user)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "author_email",
            "Author Email",
            "Email of the author (default: authenticated user's email)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

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
            "commit_sha",
            "Commit SHA",
            "SHA of the commit",
            VariableType::String,
        );
        node.add_output_pin(
            "file_sha",
            "File SHA",
            "New SHA of the file",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(8)
                .set_governance(5)
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
        let path: String = context.evaluate_pin("path").await?;
        let source_file: Option<FlowPath> = context.evaluate_pin("source_file").await.ok();
        let content: String = context.evaluate_pin("content").await.unwrap_or_default();
        let message: String = context.evaluate_pin("message").await?;
        let sha: String = context.evaluate_pin("sha").await.unwrap_or_default();
        let branch: String = context.evaluate_pin("branch").await.unwrap_or_default();
        let committer_name: String = context
            .evaluate_pin("committer_name")
            .await
            .unwrap_or_default();
        let committer_email: String = context
            .evaluate_pin("committer_email")
            .await
            .unwrap_or_default();
        let author_name: String = context
            .evaluate_pin("author_name")
            .await
            .unwrap_or_default();
        let author_email: String = context
            .evaluate_pin("author_email")
            .await
            .unwrap_or_default();

        if owner.is_empty() || repo.is_empty() || path.is_empty() || message.is_empty() {
            context.log_message(
                "Owner, repository, path, and commit message are required",
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/contents/{}",
            owner,
            repo,
            urlencoding::encode(&path)
        ));

        let content_bytes = if let Some(source_file) = source_file {
            source_file.get(context, false).await?
        } else {
            content.into_bytes()
        };

        let encoded_content = flow_like_types::base64::Engine::encode(
            &flow_like_types::base64::prelude::BASE64_STANDARD,
            content_bytes,
        );

        let mut request_body = json!({
            "message": message,
            "content": encoded_content
        });

        if !sha.is_empty() {
            request_body["sha"] = json!(sha);
        }

        if !branch.is_empty() {
            request_body["branch"] = json!(branch);
        }

        if !committer_name.is_empty() && !committer_email.is_empty() {
            request_body["committer"] = json!({
                "name": committer_name,
                "email": committer_email
            });
        }

        if !author_name.is_empty() && !author_email.is_empty() {
            request_body["author"] = json!({
                "name": author_name,
                "email": author_email
            });
        }

        let client = reqwest::Client::new();
        let response = client
            .put(&url)
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

                let result_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let commit_sha = result_json["commit"]["sha"].as_str().unwrap_or_default();
                let file_sha = result_json["content"]["sha"].as_str().unwrap_or_default();

                let action = if sha.is_empty() { "Created" } else { "Updated" };
                context.log_message(&format!("{} file: {}", action, path), LogLevel::Info);

                context
                    .set_pin_value("commit_sha", json!(commit_sha))
                    .await?;
                context.set_pin_value("file_sha", json!(file_sha)).await?;
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
// Delete File Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeleteGitHubFileNode {}

impl DeleteGitHubFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteGitHubFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_delete_file",
            "Delete File",
            "Delete a file from a repository",
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
            "path",
            "Path",
            "Path to file in the repository",
            VariableType::String,
        );
        node.add_input_pin(
            "sha",
            "SHA",
            "SHA of the file to delete (get from 'Get File Contents')",
            VariableType::String,
        );
        node.add_input_pin(
            "message",
            "Commit Message",
            "Commit message",
            VariableType::String,
        );

        node.add_input_pin(
            "branch",
            "Branch",
            "Branch to delete from (default: default branch)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "committer_name",
            "Committer Name",
            "Name of the committer (default: authenticated user)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "committer_email",
            "Committer Email",
            "Email of the committer (default: authenticated user's email)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "author_name",
            "Author Name",
            "Name of the author (default: authenticated user)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "author_email",
            "Author Email",
            "Email of the author (default: authenticated user's email)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

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
            "commit_sha",
            "Commit SHA",
            "SHA of the delete commit",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(9)
                .set_governance(5)
                .set_reliability(8)
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
        let path: String = context.evaluate_pin("path").await?;
        let sha: String = context.evaluate_pin("sha").await?;
        let message: String = context.evaluate_pin("message").await?;
        let branch: String = context.evaluate_pin("branch").await.unwrap_or_default();
        let committer_name: String = context
            .evaluate_pin("committer_name")
            .await
            .unwrap_or_default();
        let committer_email: String = context
            .evaluate_pin("committer_email")
            .await
            .unwrap_or_default();
        let author_name: String = context
            .evaluate_pin("author_name")
            .await
            .unwrap_or_default();
        let author_email: String = context
            .evaluate_pin("author_email")
            .await
            .unwrap_or_default();

        if owner.is_empty()
            || repo.is_empty()
            || path.is_empty()
            || sha.is_empty()
            || message.is_empty()
        {
            context.log_message(
                "Owner, repository, path, SHA, and commit message are required",
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/contents/{}",
            owner,
            repo,
            urlencoding::encode(&path)
        ));

        let mut request_body = json!({
            "message": message,
            "sha": sha
        });

        if !branch.is_empty() {
            request_body["branch"] = json!(branch);
        }

        if !committer_name.is_empty() && !committer_email.is_empty() {
            request_body["committer"] = json!({
                "name": committer_name,
                "email": committer_email
            });
        }

        if !author_name.is_empty() && !author_email.is_empty() {
            request_body["author"] = json!({
                "name": author_name,
                "email": author_email
            });
        }

        let client = reqwest::Client::new();
        let response = client
            .delete(&url)
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

                let result_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let commit_sha = result_json["commit"]["sha"].as_str().unwrap_or_default();

                context.log_message(&format!("Deleted file: {}", path), LogLevel::Info);

                context
                    .set_pin_value("commit_sha", json!(commit_sha))
                    .await?;
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
// Download File Node (Raw Content)
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DownloadGitHubFileNode {}

impl DownloadGitHubFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DownloadGitHubFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_download_file",
            "Download File",
            "Download raw file content from a repository (for large files)",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");
        node.set_version(2);

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
            "path",
            "Path",
            "Path to file in the repository",
            VariableType::String,
        );

        node.add_input_pin(
            "ref",
            "Ref",
            "Branch, tag, or commit SHA (default: default branch)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "output_path",
            "Output Path",
            "FlowPath to write the downloaded file into",
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

        node.add_output_pin(
            "written_path",
            "Path",
            "Written file path",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin("size", "Size", "File size in bytes", VariableType::Integer);

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(9)
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
        let path: String = context.evaluate_pin("path").await?;
        let ref_param: String = context.evaluate_pin("ref").await.unwrap_or_default();
        let output_path: FlowPath = context.evaluate_pin("output_path").await?;

        if owner.is_empty() || repo.is_empty() || path.is_empty() {
            context.log_message("Owner, repository, and path are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let mut url = format!(
            "/repos/{}/{}/contents/{}",
            owner,
            repo,
            urlencoding::encode(&path)
        );

        if !ref_param.is_empty() {
            url.push_str(&format!("?ref={}", urlencoding::encode(&ref_param)));
        }

        let full_url = provider.api_url(&url);

        let client = reqwest::Client::new();
        let response = client
            .get(&full_url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/vnd.github.raw+json")
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

                context.log_message(
                    &format!("Downloaded file: {} ({} bytes)", path, size),
                    LogLevel::Info,
                );

                context
                    .set_pin_value("written_path", json!(output_path))
                    .await?;
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
