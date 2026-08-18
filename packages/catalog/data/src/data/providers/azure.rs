use crate::data::providers::util::get_pin_string_value;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores, remove_pin_by_name},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};

pub const AZ_ACCOUNT_KEY: &str = "account_key";
pub const AZ_SAS_TOKEN: &str = "sas_token";
pub const AZ_CONNECTION_STRING: &str = "connection_string";
pub const AZ_CLIENT_SECRET: &str = "client_secret";
pub const AZ_MANAGED_IDENTITY: &str = "managed_identity";
pub const AZ_WORKLOAD_IDENTITY: &str = "workload_identity";
pub const AZ_AZURE_CLI: &str = "azure_cli";
/// Entra ID (Azure AD) OAuth 2.0 bearer token. Wired to `object_store` via
/// `MicrosoftAzureBuilder::with_bearer_token_authorization`.
pub const AZ_OAUTH: &str = "oauth";

pub const AZ_AUTH_MODES: &[&str] = &[
    AZ_ACCOUNT_KEY,
    AZ_SAS_TOKEN,
    AZ_CONNECTION_STRING,
    AZ_CLIENT_SECRET,
    AZ_MANAGED_IDENTITY,
    AZ_WORKLOAD_IDENTITY,
    AZ_AZURE_CLI,
    AZ_OAUTH,
];

/// Typed credential struct emitted by `AzureProviderNode`.
///
/// Consumer nodes (Azure Blob, ADLS, Cosmos, ...) take a single `AzureProvider`
/// input pin instead of defining credential pins themselves.
#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AzureProvider {
    pub auth_mode: String,
    /// Storage account name (used by account_key / sas / managed_identity / workload_identity for Blob).
    pub account: Option<String>,
    pub access_key: Option<String>,
    pub sas_token: Option<String>,
    pub connection_string: Option<String>,
    // Service principal (client_secret mode)
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    /// Entra ID bearer token (used when auth_mode is 'oauth').
    pub bearer_token: Option<String>,
    // Optional endpoint override (Azurite, sovereign clouds)
    pub endpoint: Option<String>,
}

impl AzureProvider {
    /// Return `true` when this provider resolves credentials from the host
    /// (IMDS managed identity, AKS federated token, cached `az login`) rather
    /// than from values carried in the struct.
    ///
    /// A `connection_string` without an `AccountKey` counts too: the builder is
    /// left credential-less and object_store falls back to IMDS.
    pub fn relies_on_env_chain(&self) -> bool {
        match self.auth_mode.as_str() {
            AZ_MANAGED_IDENTITY | AZ_WORKLOAD_IDENTITY | AZ_AZURE_CLI => true,
            AZ_CONNECTION_STRING => self
                .connection_string
                .as_deref()
                .is_none_or(|cs| parse_connection_string(cs).1.is_none()),
            _ => false,
        }
    }

    /// Apply this provider's credentials to a `MicrosoftAzureBuilder`.
    ///
    /// The caller supplies the builder with `account` + `container` already set
    /// (those are consumer-level concerns). This helper only wires auth.
    ///
    /// Host-resolved modes are refused when running server-side — see
    /// [`ExecutionEnvironment::ensure_no_ambient_credentials`](flow_like::flow::execution::ExecutionEnvironment::ensure_no_ambient_credentials).
    pub fn apply_to_azure_builder(
        &self,
        context: &ExecutionContext,
        builder: flow_like_storage::object_store::azure::MicrosoftAzureBuilder,
    ) -> flow_like_types::Result<flow_like_storage::object_store::azure::MicrosoftAzureBuilder>
    {
        use flow_like_storage::object_store::azure::{AzureConfigKey, MicrosoftAzureBuilder};

        if self.relies_on_env_chain() {
            context
                .execution_environment()
                .ensure_no_ambient_credentials("AzureProvider", &self.auth_mode)?;
        }

        let mut b: MicrosoftAzureBuilder = builder;
        if let Some(acc) = &self.account {
            b = b.with_account(acc);
        }
        if let Some(endpoint) = &self.endpoint {
            b = b.with_endpoint(endpoint.clone());
        }

        match self.auth_mode.as_str() {
            AZ_ACCOUNT_KEY => {
                let key = self.access_key.as_deref().ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "AzureProvider: access_key is required for 'account_key' auth"
                    )
                })?;
                b = b.with_access_key(key);
            }
            AZ_SAS_TOKEN => {
                let raw = self.sas_token.as_deref().ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "AzureProvider: sas_token is required for 'sas_token' auth"
                    )
                })?;
                let trimmed = raw.trim_start_matches('?');
                let pairs: Vec<(String, String)> = trimmed
                    .split('&')
                    .filter_map(|kv| {
                        let mut it = kv.splitn(2, '=');
                        match (it.next(), it.next()) {
                            (Some(k), Some(v)) if !k.is_empty() => {
                                Some((k.to_string(), v.to_string()))
                            }
                            _ => None,
                        }
                    })
                    .collect();
                b = b.with_sas_authorization(pairs);
            }
            AZ_CONNECTION_STRING => {
                let cs = self.connection_string.as_deref().ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "AzureProvider: connection_string is required for 'connection_string' auth"
                    )
                })?;
                // object_store parses connection strings via an env var; we emulate
                // by extracting AccountName / AccountKey ourselves.
                let (acc, key) = parse_connection_string(cs);
                if let Some(a) = acc {
                    b = b.with_account(a);
                }
                if let Some(k) = key {
                    b = b.with_access_key(k);
                }
            }
            AZ_CLIENT_SECRET => {
                let tenant = self.tenant_id.as_deref().ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "AzureProvider: tenant_id is required for 'client_secret' auth"
                    )
                })?;
                let client = self.client_id.as_deref().ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "AzureProvider: client_id is required for 'client_secret' auth"
                    )
                })?;
                let secret = self.client_secret.as_deref().ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "AzureProvider: client_secret is required for 'client_secret' auth"
                    )
                })?;
                b = b.with_client_secret_authorization(client, tenant, secret);
            }
            AZ_MANAGED_IDENTITY => {
                if let Some(cid) = &self.client_id {
                    b = b.with_config(AzureConfigKey::ClientId, cid);
                }
                // No explicit builder method: leaving credentials unset causes
                // object_store to fall back to IMDS managed identity.
            }
            AZ_WORKLOAD_IDENTITY => {
                // Federated token file is typically provided via
                // AZURE_FEDERATED_TOKEN_FILE env var on AKS; leaving it unset lets
                // the default credential chain resolve it.
            }
            AZ_AZURE_CLI => {
                b = b.with_use_azure_cli(true);
            }
            AZ_OAUTH => {
                let token = self.bearer_token.as_deref().ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "AzureProvider: bearer_token is required for 'oauth' auth"
                    )
                })?;
                b = b.with_bearer_token_authorization(token);
            }
            other => {
                return Err(flow_like_types::anyhow!(
                    "AzureProvider: unknown auth_mode '{}'",
                    other
                ));
            }
        }

        Ok(b)
    }
}

fn parse_connection_string(cs: &str) -> (Option<String>, Option<String>) {
    let mut account = None;
    let mut key = None;
    for part in cs.split(';') {
        let mut it = part.splitn(2, '=');
        if let (Some(k), Some(v)) = (it.next(), it.next()) {
            match k.trim() {
                "AccountName" => account = Some(v.trim().to_string()),
                "AccountKey" => key = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    (account, key)
}

#[crate::register_node]
#[derive(Default)]
pub struct AzureProviderNode {}

impl AzureProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AzureProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_azure_provider",
            "Azure Provider",
            "Build an Azure credential struct. Supports storage account key, SAS token, full connection string, service-principal (tenant/client/secret), managed identity, workload identity and Azure CLI cached tokens. Emits an AzureProvider that any Azure-aware node (Blob, ADLS, ...) can consume.",
            "Data/Providers",
        );
        node.add_icon("/flow/icons/cloud.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        add_auth_mode_pin(&mut node);

        // Account is shared across the storage-oriented modes — keep it visible.
        node.add_input_pin(
            "account",
            "Storage Account",
            "Azure storage account name (for Blob / ADLS / managed-identity-on-storage flows)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Override endpoint (Azurite, sovereign clouds). Leave empty for Azure public cloud.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        // Default mode `account_key` -> show access_key pin.
        add_access_key_pin(&mut node);

        node.add_output_pin(
            "exec_out",
            "Done",
            "Provider built",
            VariableType::Execution,
        );
        node.add_output_pin(
            "provider",
            "Provider",
            "Azure provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<AzureProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.scores = Some(NodeScores {
            privacy: 5,
            security: 7,
            performance: 10,
            governance: 8,
            reliability: 9,
            cost: 10,
        });
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let auth_mode: String = context
            .evaluate_pin("auth_mode")
            .await
            .unwrap_or_else(|_| AZ_ACCOUNT_KEY.to_string());

        if !AZ_AUTH_MODES.iter().any(|m| *m == auth_mode) {
            return Err(flow_like_types::anyhow!(
                "Unknown Azure auth_mode: '{}'. Expected one of {:?}",
                auth_mode,
                AZ_AUTH_MODES
            ));
        }

        if matches!(
            auth_mode.as_str(),
            AZ_MANAGED_IDENTITY | AZ_WORKLOAD_IDENTITY | AZ_AZURE_CLI
        ) {
            context
                .execution_environment()
                .ensure_no_ambient_credentials("AzureProvider", &auth_mode)?;
        }

        let account = context
            .evaluate_pin::<String>("account")
            .await
            .ok()
            .and_then(non_empty);
        let endpoint = context
            .evaluate_pin::<String>("endpoint")
            .await
            .ok()
            .and_then(non_empty);

        let access_key = if auth_mode.as_str() == AZ_ACCOUNT_KEY {
            context
                .evaluate_pin::<String>("access_key")
                .await
                .ok()
                .and_then(non_empty)
        } else {
            None
        };

        let sas_token = if auth_mode.as_str() == AZ_SAS_TOKEN {
            context
                .evaluate_pin::<String>("sas_token")
                .await
                .ok()
                .and_then(non_empty)
        } else {
            None
        };

        let connection_string = if auth_mode.as_str() == AZ_CONNECTION_STRING {
            context
                .evaluate_pin::<String>("connection_string")
                .await
                .ok()
                .and_then(non_empty)
        } else {
            None
        };

        let bearer_token = if auth_mode.as_str() == AZ_OAUTH {
            context
                .evaluate_pin::<String>("bearer_token")
                .await
                .ok()
                .and_then(non_empty)
        } else {
            None
        };

        let (tenant_id, client_id, client_secret) = if auth_mode.as_str() == AZ_CLIENT_SECRET {
            (
                context
                    .evaluate_pin::<String>("tenant_id")
                    .await
                    .ok()
                    .and_then(non_empty),
                context
                    .evaluate_pin::<String>("client_id")
                    .await
                    .ok()
                    .and_then(non_empty),
                context
                    .evaluate_pin::<String>("client_secret")
                    .await
                    .ok()
                    .and_then(non_empty),
            )
        } else {
            (None, None, None)
        };

        let provider = AzureProvider {
            auth_mode,
            account,
            access_key,
            sas_token,
            connection_string,
            tenant_id,
            client_id,
            client_secret,
            bearer_token,
            endpoint,
        };

        context.set_pin_value("provider", json!(provider)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let auth_mode = get_pin_string_value(node, "auth_mode");
        sync_auth_mode_pins(node, &auth_mode);
    }
}

fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s) }
}

fn add_auth_mode_pin(node: &mut Node) {
    node.add_input_pin(
        "auth_mode",
        "Auth Mode",
        "How to authenticate: 'account_key' (storage key), 'sas_token', 'connection_string', 'client_secret' (service principal), 'managed_identity', 'workload_identity' (AKS federated), 'azure_cli' (cached az login), 'oauth' (Entra ID bearer token)",
        VariableType::String,
    )
    .set_options(
        PinOptions::new()
            .set_valid_values(AZ_AUTH_MODES.iter().map(|s| s.to_string()).collect())
            .build(),
    )
    .set_default_value(Some(json!(AZ_ACCOUNT_KEY)));
}

fn add_access_key_pin(node: &mut Node) {
    node.add_input_pin(
        "access_key",
        "Access Key",
        "Storage account key (used when auth_mode is 'account_key')",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn add_sas_token_pin(node: &mut Node) {
    node.add_input_pin(
        "sas_token",
        "SAS Token",
        "Shared Access Signature token (used when auth_mode is 'sas_token')",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn add_connection_string_pin(node: &mut Node) {
    node.add_input_pin(
        "connection_string",
        "Connection String",
        "Full Azure connection string (used when auth_mode is 'connection_string')",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn add_bearer_token_pin(node: &mut Node) {
    node.add_input_pin(
        "bearer_token",
        "Bearer Token",
        "Entra ID (Azure AD) OAuth 2.0 access token (used when auth_mode is 'oauth'). Typically sourced from the Microsoft identity platform or the `az account get-access-token` CLI.",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn add_service_principal_pins(node: &mut Node) {
    node.add_input_pin(
        "tenant_id",
        "Tenant ID",
        "Azure AD tenant ID",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "client_id",
        "Client ID",
        "Azure AD application (client) ID",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "client_secret",
        "Client Secret",
        "Azure AD application client secret",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn remove_service_principal_pins(node: &mut Node) {
    remove_pin_by_name(node, "tenant_id");
    remove_pin_by_name(node, "client_id");
    remove_pin_by_name(node, "client_secret");
}

fn sync_auth_mode_pins(node: &mut Node, auth_mode: &str) {
    let mode = if auth_mode.is_empty() {
        AZ_ACCOUNT_KEY
    } else {
        auth_mode
    };

    let want_key = mode == AZ_ACCOUNT_KEY;
    let want_sas = mode == AZ_SAS_TOKEN;
    let want_cs = mode == AZ_CONNECTION_STRING;
    let want_sp = mode == AZ_CLIENT_SECRET;
    let want_oauth = mode == AZ_OAUTH;

    let has_key = node.get_pin_by_name("access_key").is_some();
    let has_sas = node.get_pin_by_name("sas_token").is_some();
    let has_cs = node.get_pin_by_name("connection_string").is_some();
    let has_sp = node.get_pin_by_name("tenant_id").is_some();
    let has_oauth = node.get_pin_by_name("bearer_token").is_some();

    if want_key && !has_key {
        add_access_key_pin(node);
    }
    if !want_key && has_key {
        remove_pin_by_name(node, "access_key");
    }

    if want_sas && !has_sas {
        add_sas_token_pin(node);
    }
    if !want_sas && has_sas {
        remove_pin_by_name(node, "sas_token");
    }

    if want_cs && !has_cs {
        add_connection_string_pin(node);
    }
    if !want_cs && has_cs {
        remove_pin_by_name(node, "connection_string");
    }

    if want_sp && !has_sp {
        add_service_principal_pins(node);
    }
    if !want_sp && has_sp {
        remove_service_principal_pins(node);
    }

    if want_oauth && !has_oauth {
        add_bearer_token_pin(node);
    }
    if !want_oauth && has_oauth {
        remove_pin_by_name(node, "bearer_token");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;

    #[test]
    fn test_node_structure() {
        let node = AzureProviderNode::new().get_node();
        assert_eq!(node.name, "data_azure_provider");
        assert_eq!(node.friendly_name, "Azure Provider");
        assert_eq!(node.category, "Data/Providers");
    }

    #[test]
    fn test_provider_output_is_schema_typed() {
        let node = AzureProviderNode::new().get_node();
        let out = node
            .pins
            .values()
            .find(|p| p.name == "provider" && p.pin_type == PinType::Output)
            .expect("provider output pin");
        assert_eq!(out.data_type, VariableType::Struct);
        assert!(out.schema.is_some());
    }

    #[test]
    fn test_default_shows_account_key_pin() {
        let node = AzureProviderNode::new().get_node();
        assert!(node.get_pin_by_name("access_key").is_some());
        assert!(node.get_pin_by_name("sas_token").is_none());
        assert!(node.get_pin_by_name("connection_string").is_none());
        assert!(node.get_pin_by_name("tenant_id").is_none());
    }

    #[test]
    fn test_parse_connection_string_extracts_account_and_key() {
        let cs = "DefaultEndpointsProtocol=https;AccountName=myacct;AccountKey=abc==;EndpointSuffix=core.windows.net";
        let (account, key) = parse_connection_string(cs);
        assert_eq!(account.as_deref(), Some("myacct"));
        assert_eq!(key.as_deref(), Some("abc=="));
    }

    #[test]
    fn test_oauth_mode_adds_bearer_token_pin() {
        let mut node = AzureProviderNode::new().get_node();
        sync_auth_mode_pins(&mut node, AZ_OAUTH);
        let pin = node
            .get_pin_by_name("bearer_token")
            .expect("bearer_token pin present for oauth mode");
        assert_eq!(pin.data_type, VariableType::String);
        sync_auth_mode_pins(&mut node, AZ_ACCOUNT_KEY);
        assert!(node.get_pin_by_name("bearer_token").is_none());
    }

    #[test]
    fn test_sync_switches_and_is_idempotent() {
        let mut node = AzureProviderNode::new().get_node();

        sync_auth_mode_pins(&mut node, AZ_CLIENT_SECRET);
        assert!(node.get_pin_by_name("access_key").is_none());
        assert!(node.get_pin_by_name("tenant_id").is_some());
        let id_before = node.get_pin_by_name("tenant_id").unwrap().id.clone();

        sync_auth_mode_pins(&mut node, AZ_CLIENT_SECRET);
        let id_after = node.get_pin_by_name("tenant_id").unwrap().id.clone();
        assert_eq!(id_before, id_after);

        sync_auth_mode_pins(&mut node, AZ_MANAGED_IDENTITY);
        assert!(node.get_pin_by_name("tenant_id").is_none());
        assert!(node.get_pin_by_name("access_key").is_none());
    }
}
