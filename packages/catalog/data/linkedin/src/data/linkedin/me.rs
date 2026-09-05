use crate::data::linkedin::provider::{LINKEDIN_PROVIDER_ID, LinkedInProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinkedInMe {
    pub sub: String,
    pub name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub picture: Option<String>,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub locale: Option<String>,
}

fn parse_linkedin_me(value: &Value) -> Option<LinkedInMe> {
    let obj = value.as_object()?;

    Some(LinkedInMe {
        sub: obj.get("sub")?.as_str()?.to_string(),
        name: obj.get("name").and_then(|v| v.as_str()).map(String::from),
        given_name: obj
            .get("given_name")
            .and_then(|v| v.as_str())
            .map(String::from),
        family_name: obj
            .get("family_name")
            .and_then(|v| v.as_str())
            .map(String::from),
        picture: obj
            .get("picture")
            .and_then(|v| v.as_str())
            .map(String::from),
        email: obj.get("email").and_then(|v| v.as_str()).map(String::from),
        email_verified: obj.get("email_verified").and_then(|v| v.as_bool()),
        locale: obj
            .get("locale")
            .and_then(|v| v.as_object())
            .and_then(|l| l.get("language"))
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct GetMeNode {}

impl GetMeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetMeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_linkedin_get_me",
            "Get Me",
            "Get the current authenticated user's LinkedIn profile information",
            "Data/LinkedIn",
        );
        node.set_flowscript_name("linkedin", "getMe");
        node.add_icon("/flow/icons/linkedin.svg");

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
            "LinkedIn provider",
            VariableType::Struct,
        )
        .set_schema::<LinkedInProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "me",
            "Me",
            "Current user's LinkedIn profile",
            VariableType::Struct,
        )
        .set_schema::<LinkedInMe>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "sub",
            "User ID",
            "The user's unique LinkedIn ID (sub claim)",
            VariableType::String,
        );

        node.add_output_pin(
            "email",
            "Email",
            "The user's email address",
            VariableType::String,
        );

        node.add_output_pin(
            "name",
            "Name",
            "The user's display name",
            VariableType::String,
        );

        node.add_required_oauth_scopes(LINKEDIN_PROVIDER_ID, vec!["openid", "profile", "email"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(9)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: LinkedInProvider = context.evaluate_pin("provider").await?;

        let client = reqwest::Client::new();
        let url = "https://api.linkedin.com/v2/userinfo";

        let response = client
            .get(url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get current user: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let me = parse_linkedin_me(&data).ok_or_else(|| {
            flow_like_types::anyhow!("Failed to parse user data from LinkedIn API")
        })?;

        context.set_pin_value("me", json!(me.clone())).await?;
        context.set_pin_value("sub", json!(me.sub)).await?;
        context
            .set_pin_value("email", json!(me.email.unwrap_or_default()))
            .await?;
        context
            .set_pin_value("name", json!(me.name.unwrap_or_default()))
            .await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
