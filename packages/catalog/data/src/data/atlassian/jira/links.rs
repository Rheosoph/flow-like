use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

/// Jira issue link type
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraLinkType {
    pub id: String,
    pub name: String,
    pub inward: String,
    pub outward: String,
}

fn parse_link_type(value: &Value) -> Option<JiraLinkType> {
    let obj = value.as_object()?;

    Some(JiraLinkType {
        id: obj.get("id")?.as_str()?.to_string(),
        name: obj.get("name")?.as_str()?.to_string(),
        inward: obj.get("inward")?.as_str()?.to_string(),
        outward: obj.get("outward")?.as_str()?.to_string(),
    })
}

/// Jira issue link
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraIssueLink {
    pub id: String,
    pub link_type: JiraLinkType,
    pub inward_issue: Option<String>,
    pub outward_issue: Option<String>,
}

fn parse_issue_link(value: &Value) -> Option<JiraIssueLink> {
    let obj = value.as_object()?;

    let link_type = obj.get("type").and_then(parse_link_type)?;

    Some(JiraIssueLink {
        id: obj.get("id")?.as_str()?.to_string(),
        link_type,
        inward_issue: obj
            .get("inwardIssue")
            .and_then(|i| i.get("key"))
            .and_then(|k| k.as_str())
            .map(String::from),
        outward_issue: obj
            .get("outwardIssue")
            .and_then(|i| i.get("key"))
            .and_then(|k| k.as_str())
            .map(String::from),
    })
}

/// Get all available issue link types
#[crate::register_node]
#[derive(Default)]
pub struct GetLinkTypesNode {}

impl GetLinkTypesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetLinkTypesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_link_types",
            "Get Link Types",
            "Get all available issue link types",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "getLinkTypes");
        node.add_icon("/flow/icons/jira.svg");

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "link_types",
            "Link Types",
            "Available link types",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraLinkType>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;

        let client = reqwest::Client::new();
        let url = provider.jira_api_url("/issueLinkType");

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get link types: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let link_types: Vec<JiraLinkType> = data["issueLinkTypes"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(parse_link_type)
            .collect();

        context
            .set_pin_value("link_types", json!(link_types))
            .await?;

        Ok(())
    }
}

/// Get links for an issue
#[crate::register_node]
#[derive(Default)]
pub struct GetIssueLinksNode {}

impl GetIssueLinksNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetIssueLinksNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_issue_links",
            "Get Issue Links",
            "Get all links for an issue",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "getIssueLinks");
        node.add_icon("/flow/icons/jira.svg");

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "issue_key",
            "Issue Key",
            "The issue key (e.g., PROJ-123)",
            VariableType::String,
        );

        node.add_output_pin("links", "Links", "Issue links", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<JiraIssueLink>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("count", "Count", "Number of links", VariableType::Integer);

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let issue_key: String = context.evaluate_pin("issue_key").await?;

        if issue_key.is_empty() {
            return Err(flow_like_types::anyhow!("Issue key is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!("/issue/{}?fields=issuelinks", issue_key));

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get issue links: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let links: Vec<JiraIssueLink> = data["fields"]["issuelinks"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(parse_issue_link)
            .collect();

        let count = links.len() as i64;

        context.set_pin_value("links", json!(links)).await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}

/// Create a link between two issues
#[crate::register_node]
#[derive(Default)]
pub struct CreateIssueLinkNode {}

impl CreateIssueLinkNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateIssueLinkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_create_issue_link",
            "Create Issue Link",
            "Create a link between two issues",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "createIssueLink");
        node.add_icon("/flow/icons/jira.svg");

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "link_type",
            "Link Type",
            "The name of the link type (e.g., 'Blocks', 'Cloners', 'Relates')",
            VariableType::String,
        );

        node.add_input_pin(
            "inward_issue",
            "Inward Issue",
            "The inward issue key (e.g., PROJ-123)",
            VariableType::String,
        );

        node.add_input_pin(
            "outward_issue",
            "Outward Issue",
            "The outward issue key (e.g., PROJ-456)",
            VariableType::String,
        );

        node.add_input_pin(
            "comment",
            "Comment",
            "Optional comment for the link",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the link was created successfully",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let link_type: String = context.evaluate_pin("link_type").await?;
        let inward_issue: String = context.evaluate_pin("inward_issue").await?;
        let outward_issue: String = context.evaluate_pin("outward_issue").await?;
        let comment: String = context.evaluate_pin("comment").await.unwrap_or_default();

        if link_type.is_empty() {
            return Err(flow_like_types::anyhow!("Link type is required"));
        }
        if inward_issue.is_empty() {
            return Err(flow_like_types::anyhow!("Inward issue key is required"));
        }
        if outward_issue.is_empty() {
            return Err(flow_like_types::anyhow!("Outward issue key is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url("/issueLink");

        let mut body = json!({
            "type": {
                "name": link_type
            },
            "inwardIssue": {
                "key": inward_issue
            },
            "outwardIssue": {
                "key": outward_issue
            }
        });

        if !comment.is_empty() {
            if provider.is_cloud {
                body["comment"] = json!({
                    "body": {
                        "type": "doc",
                        "version": 1,
                        "content": [
                            {
                                "type": "paragraph",
                                "content": [
                                    {
                                        "type": "text",
                                        "text": comment
                                    }
                                ]
                            }
                        ]
                    }
                });
            } else {
                body["comment"] = json!({
                    "body": comment
                });
            }
        }

        let response = client
            .post(&url)
            .header("Authorization", provider.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to create issue link: {} - {}",
                status,
                error_text
            ));
        }

        context.set_pin_value("success", json!(true)).await?;

        Ok(())
    }
}

/// Remove a link between issues
#[crate::register_node]
#[derive(Default)]
pub struct RemoveIssueLinkNode {}

impl RemoveIssueLinkNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RemoveIssueLinkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_remove_issue_link",
            "Remove Issue Link",
            "Remove a link between issues",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "removeIssueLink");
        node.add_icon("/flow/icons/jira.svg");

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "link_id",
            "Link ID",
            "The ID of the link to remove",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the link was removed successfully",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let link_id: String = context.evaluate_pin("link_id").await?;

        if link_id.is_empty() {
            return Err(flow_like_types::anyhow!("Link ID is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!("/issueLink/{}", link_id));

        let response = client
            .delete(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to remove issue link: {} - {}",
                status,
                error_text
            ));
        }

        context.set_pin_value("success", json!(true)).await?;

        Ok(())
    }
}
