use super::{
    CATEGORY, DATAVERSE_PROVIDER_ID, ENVIRONMENTS_READ_SCOPE, FLOWSCRIPT_NAMESPACE,
    GLOBAL_DISCOVERY_SCOPE, POWER_PLATFORM_PROVIDER_ID,
    agent::CopilotStudioAgent,
    http_failure,
    provider::{CopilotStudioProvider, add_provider_input_pin},
    scores,
};
use flow_like::{
    flow::{
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic},
        pin::{PinOptions, ValueType},
        variable::VariableType,
    },
    hub::Hub,
};
use flow_like_types::{
    JsonSchema, Result, Value, anyhow, async_trait,
    json::json,
    reqwest::{self, Url},
};
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const ENVIRONMENTS_URL: &str =
    "https://api.powerplatform.com/environmentmanagement/environments?api-version=2024-10-01";
const POWER_PLATFORM_HOST: &str = "api.powerplatform.com";
const GLOBAL_DISCOVERY_URL: &str = "https://globaldisco.crm.dynamics.com/api/discovery/v2.0/Instances?$select=Url,FriendlyName,EnvironmentId,UniqueName,State,TenantId";
const BOT_COLUMNS: &str = "botid,name,schemaname,publishedon,authenticationmode,accesscontrolpolicy,language,statecode,statuscode";
const MAX_PAGES: usize = 50;
const PARALLEL_ENVIRONMENTS: usize = 4;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct PowerPlatformEnvironment {
    pub id: String,
    pub display_name: String,
    pub dataverse_url: Option<String>,
    pub domain_name: Option<String>,
    pub tenant_id: Option<String>,
    pub environment_type: Option<String>,
    pub state: Option<String>,
    pub geo: Option<String>,
    pub azure_region: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct CopilotStudioAgentInfo {
    pub bot_id: String,
    pub name: String,
    pub schema_name: String,
    pub environment_id: String,
    pub environment_name: String,
    pub environment_url: String,
    pub published_on: Option<String>,
    /// None, Microsoft (integrated), Entra ID (manual) or Generic OAuth2.
    pub authentication: String,
    /// Any, Agent readers, Group membership or Any (multi-tenant).
    pub access: String,
    pub connection_string: String,
    pub agent: CopilotStudioAgent,
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()?)
}

/// Follows OData paging, but never sends the token to a host other than `allowed_host`.
async fn get_paged(
    client: &reqwest::Client,
    first_url: &str,
    token: &str,
    allowed_host: &str,
    operation: &str,
) -> Result<Vec<Value>> {
    let mut values = Vec::new();
    let mut next = Some(first_url.to_string());
    for _ in 0..MAX_PAGES {
        let Some(url) = next.take() else { break };
        let url = page_url(&url, allowed_host, operation)?;
        let mut page = get_page(client, url, token, operation).await?;
        if let Some(Value::Array(items)) = page.get_mut("value").map(Value::take) {
            values.extend(items);
        }
        next = page["@odata.nextLink"]
            .as_str()
            .or_else(|| page["@odata.nextlink"].as_str())
            .map(str::to_string);
    }
    Ok(values)
}

fn page_url(url: &str, allowed_host: &str, operation: &str) -> Result<Url> {
    let parsed =
        Url::parse(url).map_err(|error| anyhow!("{operation}: bad page URL {url}: {error}"))?;
    if parsed.scheme() != "https" || parsed.host_str() != Some(allowed_host) {
        return Err(anyhow!(
            "{operation}: refusing to follow a page link to '{}' (expected {allowed_host})",
            parsed.host_str().unwrap_or_default()
        ));
    }
    Ok(parsed)
}

async fn get_page(
    client: &reqwest::Client,
    url: Url,
    token: &str,
    operation: &str,
) -> Result<Value> {
    let response = client
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/json")
        .header("OData-Version", "4.0")
        .header("OData-MaxVersion", "4.0")
        .send()
        .await
        .map_err(|error| anyhow!("{operation} failed: {error}"))?;
    if !response.status().is_success() {
        return Err(anyhow!(http_failure(operation, response).await));
    }
    response
        .json()
        .await
        .map_err(|error| anyhow!("{operation} returned invalid JSON: {error}"))
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn parse_environment(value: &Value) -> Option<PowerPlatformEnvironment> {
    Some(PowerPlatformEnvironment {
        id: string_field(value, "id")?,
        display_name: string_field(value, "displayName").unwrap_or_default(),
        dataverse_url: string_field(value, "url"),
        domain_name: string_field(value, "domainName"),
        tenant_id: string_field(value, "tenantId"),
        environment_type: string_field(value, "type"),
        state: string_field(value, "state"),
        geo: string_field(value, "geo"),
        azure_region: string_field(value, "azureRegion"),
    })
}

async fn fail(context: &mut ExecutionContext, message: &str) -> Result<()> {
    context.log_message(message, LogLevel::Error);
    context
        .set_pin_value("error_message", json!(message))
        .await?;
    context.activate_exec_pin("error").await?;
    Ok(())
}

// =============================================================================
// List Power Platform Environments
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CopilotStudioListEnvironmentsNode {}

impl CopilotStudioListEnvironmentsNode {
    async fn list(context: &mut ExecutionContext) -> Result<Vec<PowerPlatformEnvironment>> {
        let provider: CopilotStudioProvider = context.evaluate_pin("provider").await?;
        let filter: String = context.evaluate_pin("filter").await.unwrap_or_default();
        let mut url = Url::parse(ENVIRONMENTS_URL)?;
        if !filter.trim().is_empty() {
            url.query_pairs_mut().append_pair("$filter", filter.trim());
        }
        let values = get_paged(
            &http_client()?,
            url.as_str(),
            &provider.access_token,
            POWER_PLATFORM_HOST,
            "Listing Power Platform environments",
        )
        .await?;
        Ok(values.iter().filter_map(parse_environment).collect())
    }
}

#[async_trait]
impl NodeLogic for CopilotStudioListEnvironmentsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_studio_list_environments",
            "List Power Platform Environments",
            "List the Power Platform environments the signed-in user can access (Power Platform API, preview). Needs the EnvironmentManagement.Environments.Read permission in addition to Copilot Studio's.",
            CATEGORY,
        );
        node.set_flowscript_name(FLOWSCRIPT_NAMESPACE, "listEnvironments");
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_input_pin(&mut node);
        node.add_input_pin(
            "filter",
            "Filter",
            "Optional OData $filter on dataverseId, type, geo, state, environmentGroupId or domainName, e.g. type eq 'Production'",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Environments listed",
            VariableType::Execution,
        );
        node.add_output_pin("error", "Error", "Listing failed", VariableType::Execution);
        node.add_output_pin(
            "environments",
            "Environments",
            "Environments available to the user",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<PowerPlatformEnvironment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Why listing failed",
            VariableType::String,
        );

        node.add_required_oauth_scopes(POWER_PLATFORM_PROVIDER_ID, vec![ENVIRONMENTS_READ_SCOPE]);
        node.set_long_running(true);
        node.set_scores(scores(7, 7, 10));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;
        match Self::list(context).await {
            Ok(environments) => {
                context
                    .set_pin_value("environments", json!(environments))
                    .await?;
                context.set_pin_value("error_message", json!("")).await?;
                context.activate_exec_pin("exec_out").await?;
                Ok(())
            }
            Err(error) => fail(context, &format!("{error:#}")).await,
        }
    }
}

// =============================================================================
// List Copilot Studio Agents
// =============================================================================

#[derive(Debug, Clone)]
struct DataverseInstance {
    url: String,
    host: String,
    environment_id: String,
    name: String,
}

/// Commercial Dataverse org hosts look like `contoso.crm4.dynamics.com`.
fn dataverse_host(url: &str) -> Result<String> {
    let parsed = Url::parse(url.trim())
        .map_err(|error| anyhow!("'{url}' is not a Dataverse URL: {error}"))?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if parsed.scheme() != "https" || !is_dataverse_org_host(&host) {
        return Err(anyhow!(
            "'{url}' is not a commercial Dataverse environment URL (https://<org>.crm[N].dynamics.com)"
        ));
    }
    Ok(host)
}

fn is_dataverse_org_host(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    let [org, region, "dynamics", "com"] = labels.as_slice() else {
        return false;
    };
    let org_valid = !org.is_empty() && org.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let region_valid = region
        .strip_prefix("crm")
        .is_some_and(|digits| digits.chars().all(|c| c.is_ascii_digit()));
    org_valid && region_valid
}

fn parse_instance(value: &Value) -> Option<DataverseInstance> {
    if value["State"].as_i64().is_some_and(|state| state != 0) {
        return None;
    }
    let url = string_field(value, "Url")?;
    let host = dataverse_host(&url).ok()?;
    Some(DataverseInstance {
        url: format!("https://{host}"),
        host,
        environment_id: string_field(value, "EnvironmentId")?,
        name: string_field(value, "FriendlyName")
            .or_else(|| string_field(value, "UniqueName"))
            .unwrap_or_default(),
    })
}

fn authentication_label(mode: Option<i64>) -> &'static str {
    match mode {
        Some(1) => "None",
        Some(2) => "Microsoft",
        Some(3) => "Entra ID (manual)",
        Some(4) => "Generic OAuth2",
        _ => "Unspecified",
    }
}

fn access_label(policy: Option<i64>) -> &'static str {
    match policy {
        Some(1) => "Agent readers",
        Some(2) => "Group membership",
        Some(3) => "Any (multi-tenant)",
        _ => "Any",
    }
}

fn parse_bot(instance: &DataverseInstance, value: &Value) -> Option<CopilotStudioAgentInfo> {
    let schema_name = string_field(value, "schemaname")?;
    let agent =
        CopilotStudioAgent::from_environment(&instance.environment_id, &schema_name).ok()?;
    let connection_string = agent.conversation_url(None).ok()?.to_string();
    Some(CopilotStudioAgentInfo {
        bot_id: string_field(value, "botid")?,
        name: string_field(value, "name").unwrap_or_else(|| schema_name.clone()),
        schema_name,
        environment_id: instance.environment_id.clone(),
        environment_name: instance.name.clone(),
        environment_url: instance.url.clone(),
        published_on: string_field(value, "publishedon"),
        authentication: authentication_label(value["authenticationmode"].as_i64()).to_string(),
        access: access_label(value["accesscontrolpolicy"].as_i64()).to_string(),
        connection_string,
        agent,
    })
}

struct DataverseSession {
    client: reqwest::Client,
    token_url: String,
    client_id: String,
    refresh_token: String,
}

impl DataverseSession {
    /// Reads the Hub's `microsoft_dataverse` app registration so the refresh token is redeemed by
    /// the same client that obtained it.
    async fn open(context: &ExecutionContext, refresh_token: String) -> Result<Self> {
        let hub = Hub::new(&context.profile.hub, context.app_state.http_client.clone())
            .await
            .map_err(|error| {
                anyhow!(
                    "Could not load the Hub OAuth configuration from '{}': {error}",
                    context.profile.hub
                )
            })?;
        let config = hub.oauth_providers.get(DATAVERSE_PROVIDER_ID).ok_or_else(|| {
            anyhow!(
                "The Hub has no '{DATAVERSE_PROVIDER_ID}' OAuth provider configured, so Dataverse tokens cannot be issued"
            )
        })?;
        if config.requires_secret_proxy || config.client_secret_env.is_some() {
            return Err(anyhow!(
                "The Hub's '{DATAVERSE_PROVIDER_ID}' provider needs a client secret; per-environment Dataverse tokens need a public (PKCE) client"
            ));
        }
        Ok(Self {
            client: http_client()?,
            token_url: config.token_url.clone(),
            client_id: config.client_id.clone(),
            refresh_token,
        })
    }

    /// Entra refresh tokens are valid for every resource the user consented to for this client;
    /// the Global Discovery sign-in grants the Dynamics CRM consent every org URL needs.
    async fn environment_token(&self, instance: &DataverseInstance) -> Result<String> {
        let scope = format!("{}/user_impersonation", instance.url);
        let response = self
            .client
            .post(&self.token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", self.client_id.as_str()),
                ("refresh_token", self.refresh_token.as_str()),
                ("scope", scope.as_str()),
            ])
            .send()
            .await
            .map_err(|error| {
                anyhow!(
                    "Requesting a Dataverse token for {} failed: {error}",
                    instance.url
                )
            })?;
        if !response.status().is_success() {
            let operation = format!("Requesting a Dataverse token for {}", instance.url);
            return Err(anyhow!(http_failure(&operation, response).await));
        }
        let body: Value = response.json().await?;
        body["access_token"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("Entra returned no access_token for {}", instance.url))
    }

    async fn agents(
        &self,
        instance: &DataverseInstance,
        published_only: bool,
    ) -> Result<Vec<CopilotStudioAgentInfo>> {
        let token = self.environment_token(instance).await?;
        let mut filter = "statecode eq 0".to_string();
        if published_only {
            filter.push_str(" and publishedon ne null");
        }
        let mut url = Url::parse(&format!("{}/api/data/v9.2/bots", instance.url))?;
        url.query_pairs_mut()
            .append_pair("$select", BOT_COLUMNS)
            .append_pair("$filter", &filter);
        let operation = format!("Listing agents in {}", instance.name);
        let values = get_paged(
            &self.client,
            url.as_str(),
            &token,
            &instance.host,
            &operation,
        )
        .await?;
        Ok(values
            .iter()
            .filter_map(|value| parse_bot(instance, value))
            .collect())
    }
}

type EnvironmentAgents = (DataverseInstance, Result<Vec<CopilotStudioAgentInfo>>);

impl DataverseSession {
    async fn agents_in(
        &self,
        instances: Vec<DataverseInstance>,
        published_only: bool,
    ) -> Vec<EnvironmentAgents> {
        stream::iter(instances)
            .map(|instance| async move {
                let agents = self.agents(&instance, published_only).await;
                (instance, agents)
            })
            .buffer_unordered(PARALLEL_ENVIRONMENTS)
            .collect()
            .await
    }
}

/// Environments the signed-in user can open, optionally narrowed to one Dataverse host.
async fn discover_instances(
    discovery_token: &str,
    requested_host: Option<&str>,
) -> Result<Vec<DataverseInstance>> {
    let discovered = get_paged(
        &http_client()?,
        GLOBAL_DISCOVERY_URL,
        discovery_token,
        "globaldisco.crm.dynamics.com",
        "Discovering Dataverse environments",
    )
    .await?;
    let instances = discovered
        .iter()
        .filter_map(parse_instance)
        .filter(|instance| requested_host.is_none_or(|host| instance.host == host));
    let instances: Vec<DataverseInstance> = instances.collect();
    if let Some(host) = requested_host
        && instances.is_empty()
    {
        return Err(anyhow!(
            "Environment {host} is not among the Dataverse environments this account can access"
        ));
    }
    Ok(instances)
}

/// Per-environment failures (e.g. no read access to the bot table) only fail the node when
/// nothing was found or the user asked for that one environment.
fn merge_agent_results(
    context: &mut ExecutionContext,
    results: Vec<EnvironmentAgents>,
    single_environment: bool,
) -> Result<Vec<CopilotStudioAgentInfo>> {
    let mut agents = Vec::new();
    let mut failures = Vec::new();
    for (instance, result) in results {
        match result {
            Ok(found) => agents.extend(found),
            Err(error) => failures.push(format!("{}: {error:#}", instance.name)),
        }
    }
    if !failures.is_empty() && (agents.is_empty() || single_environment) {
        return Err(anyhow!(failures.join("; ")));
    }
    if !failures.is_empty() {
        context.log_message(
            &format!(
                "Skipped environments while listing Copilot Studio agents: {}",
                failures.join("; ")
            ),
            LogLevel::Warn,
        );
    }
    agents.sort_by(|a, b| {
        a.environment_name
            .cmp(&b.environment_name)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(agents)
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotStudioListAgentsNode {}

impl CopilotStudioListAgentsNode {
    async fn requested_host(context: &mut ExecutionContext) -> Result<Option<String>> {
        let environment_url: String = context
            .evaluate_pin("environment_url")
            .await
            .unwrap_or_default();
        match environment_url.trim() {
            "" => Ok(None),
            url => dataverse_host(url).map(Some),
        }
    }

    async fn list(context: &mut ExecutionContext) -> Result<Vec<CopilotStudioAgentInfo>> {
        let token = context
            .get_oauth_token(DATAVERSE_PROVIDER_ID)
            .cloned()
            .ok_or_else(|| {
                anyhow!("Microsoft Dataverse is not authenticated or its token expired. Authorize access when prompted.")
            })?;
        let published_only: bool = context.evaluate_pin("published_only").await.unwrap_or(true);
        let requested_host = Self::requested_host(context).await?;

        let instances = discover_instances(&token.access_token, requested_host.as_deref()).await?;
        if instances.is_empty() {
            return Ok(Vec::new());
        }
        let refresh_token = token.refresh_token.ok_or_else(|| {
            anyhow!("The Dataverse sign-in has no refresh token, so per-environment tokens cannot be issued; sign in again")
        })?;
        let session = DataverseSession::open(context, refresh_token).await?;
        let results = session.agents_in(instances, published_only).await;
        merge_agent_results(context, results, requested_host.is_some())
    }
}

#[async_trait]
impl NodeLogic for CopilotStudioListAgentsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_studio_list_agents",
            "List Copilot Studio Agents",
            "List the Copilot Studio agents in the Dataverse environments the user can access (Dataverse bot table). Signs in to Microsoft Dataverse separately; plain chat users may lack read access to the bot table. Each result carries a connection string and agent reference.",
            CATEGORY,
        );
        node.set_flowscript_name(FLOWSCRIPT_NAMESPACE, "listAgents");
        node.add_icon("/flow/icons/copilot.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "environment_url",
            "Environment URL",
            "Dataverse URL of one environment (https://<org>.crm.dynamics.com); leave empty for all accessible environments",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "published_only",
            "Published Only",
            "Only return agents that have been published",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Agents listed",
            VariableType::Execution,
        );
        node.add_output_pin("error", "Error", "Listing failed", VariableType::Execution);
        node.add_output_pin(
            "agents",
            "Agents",
            "Agents with their environment, connection string and agent reference",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<CopilotStudioAgentInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Why listing failed",
            VariableType::String,
        );

        node.add_oauth_provider(DATAVERSE_PROVIDER_ID);
        node.add_required_oauth_scopes(DATAVERSE_PROVIDER_ID, vec![GLOBAL_DISCOVERY_SCOPE]);
        node.set_long_running(true);
        node.set_scores(scores(7, 5, 10));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;
        match Self::list(context).await {
            Ok(agents) => {
                context.set_pin_value("agents", json!(agents)).await?;
                context.set_pin_value("error_message", json!("")).await?;
                context.activate_exec_pin("exec_out").await?;
                Ok(())
            }
            Err(error) => fail(context, &format!("{error:#}")).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_commercial_dataverse_org_hosts_are_accepted() {
        assert_eq!(
            dataverse_host("https://contoso.crm4.dynamics.com/").unwrap(),
            "contoso.crm4.dynamics.com"
        );
        assert_eq!(
            dataverse_host("https://Contoso-Dev.crm.dynamics.com").unwrap(),
            "contoso-dev.crm.dynamics.com"
        );
        for url in [
            "http://contoso.crm.dynamics.com",
            "https://contoso.api.crm.dynamics.com",
            "https://contoso.crm.dynamics.com.evil.com",
            "https://globaldisco.crm.dynamics.com.evil",
            "https://contoso.crmx.dynamics.com",
            "https://contoso.crm.microsoftdynamics.us",
        ] {
            assert!(dataverse_host(url).is_err(), "{url} must be rejected");
        }
    }

    #[test]
    fn disabled_or_foreign_instances_are_skipped() {
        let enabled = json!({
            "Url": "https://contoso.crm.dynamics.com",
            "EnvironmentId": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            "FriendlyName": "Contoso",
            "State": 0
        });
        let instance = parse_instance(&enabled).unwrap();
        assert_eq!(instance.url, "https://contoso.crm.dynamics.com");
        assert_eq!(instance.name, "Contoso");

        let mut disabled = enabled.clone();
        disabled["State"] = json!(1);
        assert!(parse_instance(&disabled).is_none());

        let mut foreign = enabled;
        foreign["Url"] = json!("https://contoso.example.com");
        assert!(parse_instance(&foreign).is_none());
    }

    #[test]
    fn bots_become_agent_infos_with_a_usable_connection_string() {
        let instance = DataverseInstance {
            url: "https://contoso.crm.dynamics.com".to_string(),
            host: "contoso.crm.dynamics.com".to_string(),
            environment_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string(),
            name: "Contoso".to_string(),
        };
        let info = parse_bot(
            &instance,
            &json!({
                "botid": "11111111-2222-3333-4444-555555555555",
                "name": "Support",
                "schemaname": "cr3e1_support",
                "publishedon": "2026-09-01T10:00:00Z",
                "authenticationmode": 2,
                "accesscontrolpolicy": 1
            }),
        )
        .unwrap();
        assert_eq!(info.authentication, "Microsoft");
        assert_eq!(info.access, "Agent readers");
        let reparsed = CopilotStudioAgent::from_connection_string(&info.connection_string).unwrap();
        assert_eq!(reparsed, info.agent);
    }

    #[test]
    fn environments_parse_from_the_power_platform_shape() {
        let environment = parse_environment(&json!({
            "id": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            "displayName": "Contoso (default)",
            "url": "https://contoso.crm.dynamics.com/",
            "type": "Default",
            "state": "Ready"
        }))
        .unwrap();
        assert_eq!(environment.display_name, "Contoso (default)");
        assert_eq!(environment.environment_type.as_deref(), Some("Default"));
        assert!(parse_environment(&json!({ "displayName": "no id" })).is_none());
    }
}
