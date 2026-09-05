use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

/// Jira field metadata
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraField {
    pub id: String,
    pub key: String,
    pub name: String,
    pub is_custom: bool,
    pub searchable: bool,
    pub navigable: bool,
    pub orderable: bool,
    pub schema_type: Option<String>,
    pub schema_system: Option<String>,
    pub schema_items: Option<String>,
}

fn parse_field(value: &Value) -> Option<JiraField> {
    let obj = value.as_object()?;

    let schema = obj.get("schema").and_then(|s| s.as_object());

    Some(JiraField {
        id: obj.get("id")?.as_str()?.to_string(),
        key: obj
            .get("key")
            .and_then(|k| k.as_str())
            .unwrap_or_default()
            .to_string(),
        name: obj.get("name")?.as_str()?.to_string(),
        is_custom: obj.get("custom").and_then(|c| c.as_bool()).unwrap_or(false),
        searchable: obj
            .get("searchable")
            .and_then(|s| s.as_bool())
            .unwrap_or(true),
        navigable: obj
            .get("navigable")
            .and_then(|n| n.as_bool())
            .unwrap_or(true),
        orderable: obj
            .get("orderable")
            .and_then(|o| o.as_bool())
            .unwrap_or(false),
        schema_type: schema
            .and_then(|s| s.get("type"))
            .and_then(|t| t.as_str())
            .map(String::from),
        schema_system: schema
            .and_then(|s| s.get("system"))
            .and_then(|t| t.as_str())
            .map(String::from),
        schema_items: schema
            .and_then(|s| s.get("items"))
            .and_then(|i| i.as_str())
            .map(String::from),
    })
}

/// Get all available fields in a Jira instance
#[crate::register_node]
#[derive(Default)]
pub struct GetFieldsNode {}

impl GetFieldsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetFieldsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_fields",
            "Get Fields",
            "Get all available fields in Jira (system and custom fields)",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "getFields");
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
            "fields",
            "Fields",
            "All available fields",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraField>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "system_fields",
            "System Fields",
            "System fields only",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraField>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "custom_fields",
            "Custom Fields",
            "Custom fields only",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraField>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

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

        let client = reqwest::Client::new();
        let url = provider.jira_api_url("/field");

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get fields: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let fields: Vec<JiraField> = data
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(parse_field)
            .collect();

        let system_fields: Vec<JiraField> =
            fields.iter().filter(|f| !f.is_custom).cloned().collect();
        let custom_fields: Vec<JiraField> =
            fields.iter().filter(|f| f.is_custom).cloned().collect();

        context.set_pin_value("fields", json!(fields)).await?;
        context
            .set_pin_value("system_fields", json!(system_fields))
            .await?;
        context
            .set_pin_value("custom_fields", json!(custom_fields))
            .await?;

        Ok(())
    }
}

/// Search for fields by name or type
#[crate::register_node]
#[derive(Default)]
pub struct SearchFieldsNode {}

impl SearchFieldsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchFieldsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_search_fields",
            "Search Fields",
            "Search for Jira fields by name, type, or key",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "searchFields");
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
            "query",
            "Query",
            "Search query for field name or key",
            VariableType::String,
        );

        node.add_input_pin(
            "only_custom",
            "Only Custom",
            "Only return custom fields",
            VariableType::Boolean,
        );

        node.add_input_pin(
            "schema_type",
            "Schema Type",
            "Filter by schema type (e.g., 'string', 'array', 'option', 'user')",
            VariableType::String,
        );

        node.add_output_pin("fields", "Fields", "Matching fields", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<JiraField>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of matching fields",
            VariableType::Integer,
        );

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
        let query: String = context.evaluate_pin("query").await.unwrap_or_default();
        let only_custom: bool = context.evaluate_pin("only_custom").await.unwrap_or(false);
        let schema_type: String = context
            .evaluate_pin("schema_type")
            .await
            .unwrap_or_default();

        let client = reqwest::Client::new();
        let url = provider.jira_api_url("/field");

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get fields: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let query_lower = query.to_lowercase();

        let fields: Vec<JiraField> = data
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(parse_field)
            .filter(|f| {
                if only_custom && !f.is_custom {
                    return false;
                }
                if !schema_type.is_empty() {
                    if let Some(ref ft) = f.schema_type {
                        if !ft.to_lowercase().contains(&schema_type.to_lowercase()) {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                if query_lower.is_empty() {
                    return true;
                }
                f.name.to_lowercase().contains(&query_lower)
                    || f.id.to_lowercase().contains(&query_lower)
                    || f.key.to_lowercase().contains(&query_lower)
            })
            .collect();

        let count = fields.len() as i64;

        context.set_pin_value("fields", json!(fields)).await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}
