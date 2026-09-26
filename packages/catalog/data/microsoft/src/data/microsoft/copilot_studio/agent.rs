use super::{CATEGORY, FLOWSCRIPT_NAMESPACE, scores};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Result, anyhow, async_trait, json::json, reqwest::Url};
use serde::{Deserialize, Serialize};

pub(crate) const ENVIRONMENT_HOST_SUFFIX: &str = ".environment.api.powerplatform.com";
pub(crate) const D2E_API_VERSION: &str = "2022-03-01-preview";
const CONNECTION_STRING_HELP: &str = "Copy it from Copilot Studio → Channels → Native app → \"Microsoft 365 Agents SDK\" connection string.";

/// A validated reference to one published Copilot Studio agent.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct CopilotStudioAgent {
    /// `https://{environment host}/copilotstudio/…/bots/{schema name}/conversations`, without query.
    pub conversations_url: String,
    pub environment_host: String,
    pub schema_name: String,
}

impl CopilotStudioAgent {
    /// Parses the SDK connection string the same way the Agents SDK normalises a direct connect
    /// URL, and only accepts commercial-cloud Power Platform environment hosts so a board can never
    /// send the user's token anywhere else.
    pub fn from_connection_string(input: &str) -> Result<Self> {
        let url = parse_plain_https(input)?;
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        validate_environment_host(&host)?;
        let (path, schema_name) = agent_path(url.path())?;
        Ok(Self {
            conversations_url: format!("https://{host}{path}/conversations"),
            environment_host: host,
            schema_name,
        })
    }

    /// Builds the reference for a published agent from its environment id and schema name.
    pub fn from_environment(environment_id: &str, schema_name: &str) -> Result<Self> {
        let host = environment_host(environment_id)?;
        let schema_name = schema_name.trim();
        if schema_name.is_empty() || schema_name.contains('/') {
            return Err(anyhow!(
                "Agent schema name '{schema_name}' is empty or contains '/'"
            ));
        }
        Ok(Self {
            conversations_url: format!(
                "https://{host}/copilotstudio/dataverse-backed/authenticated/bots/{}/conversations",
                urlencoding::encode(schema_name)
            ),
            environment_host: host,
            schema_name: schema_name.to_string(),
        })
    }

    /// Re-derives the reference from its URL. Struct pins can be built by hand, so every node
    /// that sends a token re-checks the host instead of trusting the fields.
    pub(crate) fn validated(&self) -> Result<Self> {
        Self::from_connection_string(&self.conversations_url)
    }

    pub(crate) fn conversation_url(&self, conversation_id: Option<&str>) -> Result<Url> {
        let mut url = Url::parse(&self.conversations_url).map_err(|error| {
            anyhow!(
                "Agent URL '{}' is not valid: {error}",
                self.conversations_url
            )
        })?;
        if let Some(conversation_id) = conversation_id {
            url.path_segments_mut()
                .map_err(|_| anyhow!("Agent URL '{}' cannot take a path", self.conversations_url))?
                .push(conversation_id);
        }
        url.query_pairs_mut()
            .clear()
            .append_pair("api-version", D2E_API_VERSION);
        Ok(url)
    }
}

fn parse_plain_https(input: &str) -> Result<Url> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Copilot Studio connection string is empty. {CONNECTION_STRING_HELP}"
        ));
    }
    let url = Url::parse(trimmed).map_err(|error| {
        anyhow!("Copilot Studio connection string '{trimmed}' is not a valid URL ({error}). {CONNECTION_STRING_HELP}")
    })?;
    let plain = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some();
    if !plain {
        return Err(anyhow!(
            "Copilot Studio connection string '{trimmed}' must be a plain https URL without credentials or a custom port"
        ));
    }
    Ok(url)
}

/// Mirrors the SDK's `createURL`: drops a trailing slash and anything from `/conversations` on,
/// then requires `/copilotstudio/…/bots/{schema name}`.
fn agent_path(raw_path: &str) -> Result<(String, String)> {
    let path = raw_path.trim_end_matches('/');
    let path = path
        .find("/conversations")
        .map_or(path, |index| &path[..index])
        .trim_end_matches('/');
    if !path.starts_with("/copilotstudio/") {
        return Err(anyhow!(
            "Copilot Studio connection string path '{path}' does not start with /copilotstudio/. {CONNECTION_STRING_HELP}"
        ));
    }
    let encoded_schema = path
        .rsplit_once("/bots/")
        .map(|(_, schema)| schema)
        .filter(|schema| !schema.is_empty() && !schema.contains('/'))
        .ok_or_else(|| {
            anyhow!(
                "Copilot Studio connection string path '{path}' has no /bots/{{schema name}} segment. {CONNECTION_STRING_HELP}"
            )
        })?;
    let schema_name = urlencoding::decode(encoded_schema)
        .map_err(|error| {
            anyhow!("Agent schema name '{encoded_schema}' is not valid UTF-8: {error}")
        })?
        .into_owned();
    Ok((path.to_string(), schema_name))
}

/// `PowerPlatformEnvironment.getEnvironmentEndpoint` from the Agents SDK for the Prod cloud: the
/// normalised id is split two characters before its end.
pub(crate) fn environment_host(environment_id: &str) -> Result<String> {
    let normalized: String = environment_id
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|character| *character != '-')
        .collect();
    if normalized.len() < 3 || !normalized.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(anyhow!(
            "'{environment_id}' is not a Power Platform environment id"
        ));
    }
    let (prefix, suffix) = normalized.split_at(normalized.len() - 2);
    Ok(format!("{prefix}.{suffix}{ENVIRONMENT_HOST_SUFFIX}"))
}

pub(crate) fn validate_environment_host(host: &str) -> Result<()> {
    let prefix = host.strip_suffix(ENVIRONMENT_HOST_SUFFIX).ok_or_else(|| {
        anyhow!(
            "Host '{host}' is not a Copilot Studio environment host (*{ENVIRONMENT_HOST_SUFFIX}); only commercial-cloud agents are supported"
        )
    })?;
    let labels_valid = !prefix.is_empty()
        && prefix.split('.').all(|label| {
            !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if !labels_valid {
        return Err(anyhow!(
            "Host '{host}' is not a valid Copilot Studio environment host"
        ));
    }
    Ok(())
}

pub(crate) fn add_agent_input_pin(node: &mut Node) {
    node.add_input_pin(
        "agent",
        "Agent",
        "Copilot Studio agent reference (from the Copilot Studio Agent node)",
        VariableType::Struct,
    )
    .set_schema::<CopilotStudioAgent>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotStudioAgentNode {}

#[async_trait]
impl NodeLogic for CopilotStudioAgentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_studio_agent",
            "Copilot Studio Agent",
            "Reference a published Copilot Studio agent by its Microsoft 365 Agents SDK connection string (Copilot Studio → Channels → Native app). Only commercial-cloud Power Platform hosts are accepted.",
            CATEGORY,
        );
        node.set_flowscript_name(FLOWSCRIPT_NAMESPACE, "agent");
        node.add_icon("/flow/icons/copilot.svg");

        node.add_input_pin(
            "connection_string",
            "Connection String",
            "https://….environment.api.powerplatform.com/copilotstudio/dataverse-backed/authenticated/bots/{schema name}/conversations?api-version=…",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "agent",
            "Agent",
            "Validated Copilot Studio agent reference",
            VariableType::Struct,
        )
        .set_schema::<CopilotStudioAgent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_scores(scores(9, 10, 10));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let connection_string: String = context.evaluate_pin("connection_string").await?;
        let agent = CopilotStudioAgent::from_connection_string(&connection_string)?;
        context.set_pin_value("agent", json!(agent)).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SDK_URL: &str = "https://aaaaaaaabbbbccccddddeeeeeeeeee.ee.environment.api.powerplatform.com/copilotstudio/dataverse-backed/authenticated/bots/cr3e1_customerSupportAgent/conversations?api-version=2022-03-01-preview";

    #[test]
    fn connection_string_is_normalised_to_the_conversations_base() {
        let agent = CopilotStudioAgent::from_connection_string(SDK_URL).unwrap();
        assert_eq!(
            agent.conversations_url,
            "https://aaaaaaaabbbbccccddddeeeeeeeeee.ee.environment.api.powerplatform.com/copilotstudio/dataverse-backed/authenticated/bots/cr3e1_customerSupportAgent/conversations"
        );
        assert_eq!(agent.schema_name, "cr3e1_customerSupportAgent");
        assert_eq!(
            agent.environment_host,
            "aaaaaaaabbbbccccddddeeeeeeeeee.ee.environment.api.powerplatform.com"
        );
    }

    #[test]
    fn connection_string_without_conversations_suffix_or_with_trailing_slash_is_accepted() {
        let base = SDK_URL.split("/conversations").next().unwrap();
        let expected = CopilotStudioAgent::from_connection_string(SDK_URL).unwrap();
        assert_eq!(
            CopilotStudioAgent::from_connection_string(base).unwrap(),
            expected
        );
        assert_eq!(
            CopilotStudioAgent::from_connection_string(&format!("{base}/")).unwrap(),
            expected
        );
        let with_conversation = format!("{base}/conversations/abc-123");
        assert_eq!(
            CopilotStudioAgent::from_connection_string(&with_conversation).unwrap(),
            expected
        );
    }

    #[test]
    fn foreign_hosts_are_rejected_before_any_token_is_sent() {
        for url in [
            "https://evil.example.com/copilotstudio/dataverse-backed/authenticated/bots/x/conversations",
            "https://environment.api.powerplatform.com.evil.com/copilotstudio/dataverse-backed/authenticated/bots/x",
            "https://.environment.api.powerplatform.com/copilotstudio/dataverse-backed/authenticated/bots/x",
            "http://aaaa.ee.environment.api.powerplatform.com/copilotstudio/dataverse-backed/authenticated/bots/x",
            "https://user:pw@aaaa.ee.environment.api.powerplatform.com/copilotstudio/dataverse-backed/authenticated/bots/x",
            "https://aaaa.ee.environment.api.powerplatform.com:8443/copilotstudio/dataverse-backed/authenticated/bots/x",
            "https://aaaa.ee.environment.api.gov.powerplatform.microsoft.us/copilotstudio/dataverse-backed/authenticated/bots/x",
        ] {
            assert!(
                CopilotStudioAgent::from_connection_string(url).is_err(),
                "{url} must be rejected"
            );
        }
    }

    #[test]
    fn paths_outside_copilot_studio_bots_are_rejected() {
        for path in [
            "/powervirtualagents/bots/x",
            "/copilotstudio/bots/",
            "/copilotstudio/x",
        ] {
            let url = format!("https://aaaa.ee.environment.api.powerplatform.com{path}");
            assert!(
                CopilotStudioAgent::from_connection_string(&url).is_err(),
                "{url} must be rejected"
            );
        }
    }

    #[test]
    fn environment_host_matches_the_agents_sdk_prod_algorithm() {
        assert_eq!(
            environment_host("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap(),
            "aaaaaaaabbbbccccddddeeeeeeeeee.ee.environment.api.powerplatform.com"
        );
        assert_eq!(
            environment_host("Default-12345678-90AB-CDEF-1234-567890ABCDEF").unwrap(),
            "default1234567890abcdef1234567890abcd.ef.environment.api.powerplatform.com"
        );
        assert!(environment_host("not an id").is_err());
    }

    #[test]
    fn environment_reference_round_trips_through_the_connection_string_parser() {
        let agent = CopilotStudioAgent::from_environment(
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            "cr3e1_customerSupportAgent",
        )
        .unwrap();
        assert_eq!(
            agent,
            CopilotStudioAgent::from_connection_string(SDK_URL).unwrap()
        );
        assert_eq!(agent.validated().unwrap(), agent);
    }

    #[test]
    fn conversation_url_encodes_the_id_and_forces_the_api_version() {
        let agent = CopilotStudioAgent::from_connection_string(SDK_URL).unwrap();
        let url = agent.conversation_url(Some("a/b c")).unwrap();
        assert!(url.path().ends_with("/conversations/a%2Fb%20c"), "{url}");
        assert_eq!(url.query(), Some("api-version=2022-03-01-preview"));
        let start = agent.conversation_url(None).unwrap();
        assert!(start.path().ends_with("/conversations"));
    }
}
