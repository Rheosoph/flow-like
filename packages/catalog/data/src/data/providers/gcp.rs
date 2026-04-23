use crate::data::path::FlowPath;
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

pub const GCP_ADC: &str = "application_default";
pub const GCP_SA_JSON: &str = "service_account_json";
pub const GCP_SA_FILE: &str = "service_account_file";
pub const GCP_WORKLOAD: &str = "workload_identity";
pub const GCP_ACCESS_TOKEN: &str = "access_token";

pub const GCP_AUTH_MODES: &[&str] = &[
    GCP_ADC,
    GCP_SA_JSON,
    GCP_SA_FILE,
    GCP_WORKLOAD,
    GCP_ACCESS_TOKEN,
];

/// Typed credential struct emitted by `GcpProviderNode`.
///
/// `auth_mode` decides which of the optional fields are consulted at run-time.
/// The struct is the single contract between GCP-aware consumers (BigQuery,
/// GCS, etc.) and the provider node — consumer nodes should never define their
/// own GCP credential pins.
#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct GcpProvider {
    pub auth_mode: String,
    pub default_project_id: Option<String>,
    pub readonly: bool,
    pub service_account_json: Option<String>,
    pub service_account_file: Option<FlowPath>,
    pub access_token: Option<String>,
}

impl GcpProvider {
    /// Build a BigQuery client for the currently-selected auth mode.
    ///
    /// Feature-gated because `gcp-bigquery-client` is a heavy dependency only
    /// pulled in by consumers that actually talk to BigQuery.
    #[cfg(feature = "bigquery")]
    pub async fn build_bigquery_client(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<gcp_bigquery_client::Client> {
        use gcp_bigquery_client::Client;
        use gcp_bigquery_client::yup_oauth2::ServiceAccountKey;

        match self.auth_mode.as_str() {
            GCP_ADC => Client::from_application_default_credentials()
                .await
                .map_err(|e| {
                    flow_like_types::anyhow!("Application default credentials failed: {}", e)
                }),
            GCP_WORKLOAD => Client::with_workload_identity(self.readonly)
                .await
                .map_err(|e| flow_like_types::anyhow!("Workload identity auth failed: {}", e)),
            GCP_SA_JSON => {
                let raw = self.service_account_json.as_deref().unwrap_or_default();
                if raw.trim().is_empty() {
                    return Err(flow_like_types::anyhow!(
                        "GcpProvider: service_account_json is empty"
                    ));
                }
                let key: ServiceAccountKey = flow_like_types::json::from_str(raw).map_err(|e| {
                    flow_like_types::anyhow!("Invalid service account JSON key: {}", e)
                })?;
                Client::from_service_account_key(key, self.readonly)
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Service account auth failed: {}", e))
            }
            GCP_SA_FILE => {
                let path = self.service_account_file.as_ref().ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "GcpProvider: service_account_file (FlowPath) is missing"
                    )
                })?;
                let bytes = path.get(context, false).await.map_err(|e| {
                    flow_like_types::anyhow!(
                        "Failed to read service account key file from FlowPath: {}",
                        e
                    )
                })?;
                let key: ServiceAccountKey = flow_like_types::json::from_slice(&bytes)
                    .map_err(|e| {
                        flow_like_types::anyhow!("Service account key file is not valid JSON: {}", e)
                    })?;
                Client::from_service_account_key(key, self.readonly)
                    .await
                    .map_err(|e| {
                        flow_like_types::anyhow!("Service account key file auth failed: {}", e)
                    })
            }
            GCP_ACCESS_TOKEN => Err(flow_like_types::anyhow!(
                "GcpProvider: 'access_token' mode is not yet supported by the BigQuery client (gcp-bigquery-client lacks a static-token constructor). Use application_default or service_account_json instead."
            )),
            other => Err(flow_like_types::anyhow!(
                "GcpProvider: unknown auth_mode '{}'",
                other
            )),
        }
    }
}

impl GcpProvider {
    /// Apply this provider's credentials to a `GoogleCloudStorageBuilder`.
    ///
    /// The caller is expected to set `bucket_name` before/after calling this.
    /// For `service_account_file` mode, the FlowPath is read via `context` and
    /// the raw JSON is passed through as a service-account key.
    pub async fn apply_to_gcs_builder(
        &self,
        context: &mut ExecutionContext,
        builder: flow_like_storage::object_store::gcp::GoogleCloudStorageBuilder,
    ) -> flow_like_types::Result<flow_like_storage::object_store::gcp::GoogleCloudStorageBuilder> {
        use flow_like_storage::object_store::gcp::GoogleCloudStorageBuilder;

        match self.auth_mode.as_str() {
            GCP_SA_JSON => {
                let raw = self.service_account_json.as_deref().unwrap_or_default();
                if raw.trim().is_empty() {
                    return Err(flow_like_types::anyhow!(
                        "GcpProvider: service_account_json is empty"
                    ));
                }
                Ok(builder.with_service_account_key(raw))
            }
            GCP_SA_FILE => {
                let path = self.service_account_file.as_ref().ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "GcpProvider: service_account_file (FlowPath) is missing"
                    )
                })?;
                let bytes = path.get(context, false).await.map_err(|e| {
                    flow_like_types::anyhow!(
                        "Failed to read service account key file from FlowPath: {}",
                        e
                    )
                })?;
                let raw = String::from_utf8(bytes).map_err(|e| {
                    flow_like_types::anyhow!("Service account key file is not valid UTF-8: {}", e)
                })?;
                Ok(builder.with_service_account_key(raw))
            }
            GCP_ADC | GCP_WORKLOAD => {
                // Let object_store resolve via GOOGLE_APPLICATION_CREDENTIALS,
                // gcloud-cached creds, or the GCE/GKE metadata server.
                Ok(GoogleCloudStorageBuilder::from_env())
            }
            GCP_ACCESS_TOKEN => Err(flow_like_types::anyhow!(
                "GcpProvider: 'access_token' mode is not yet wired into object_store (no static-token constructor on GoogleCloudStorageBuilder). Use application_default or service_account_json instead."
            )),
            other => Err(flow_like_types::anyhow!(
                "GcpProvider: unknown auth_mode '{}'",
                other
            )),
        }
    }
}

// =============================================================================
// Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GcpProviderNode {}

impl GcpProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GcpProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_gcp_provider",
            "GCP Provider",
            "Build a Google Cloud credential struct. Supports application default credentials, service account JSON, service account key file (FlowPath), workload identity and static access tokens. Emits a GcpProvider that any GCP-aware node (BigQuery, GCS, ...) can consume.",
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

        node.add_input_pin(
            "default_project_id",
            "Default Project ID",
            "Default GCP project used by consumers that don't override it",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "readonly",
            "Read Only",
            "Request only read-only scopes when the auth mode supports it",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Start with ADC default: no mode-specific pins shown.

        node.add_output_pin(
            "exec_out",
            "Done",
            "Provider built",
            VariableType::Execution,
        );
        node.add_output_pin(
            "provider",
            "Provider",
            "GCP provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<GcpProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.scores = Some(NodeScores {
            privacy: 6,
            security: 8,
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
            .unwrap_or_else(|_| GCP_ADC.to_string());
        let default_project_id: String = context
            .evaluate_pin("default_project_id")
            .await
            .unwrap_or_default();
        let readonly: bool = context.evaluate_pin("readonly").await.unwrap_or(true);

        let service_account_json: Option<String> = match auth_mode.as_str() {
            GCP_SA_JSON => {
                let v: String = context
                    .evaluate_pin("service_account_json")
                    .await
                    .unwrap_or_default();
                if v.trim().is_empty() { None } else { Some(v) }
            }
            _ => None,
        };
        let service_account_file: Option<FlowPath> = match auth_mode.as_str() {
            GCP_SA_FILE => context.evaluate_pin("service_account_file").await.ok(),
            _ => None,
        };
        let access_token: Option<String> = match auth_mode.as_str() {
            GCP_ACCESS_TOKEN => {
                let v: String = context
                    .evaluate_pin("access_token")
                    .await
                    .unwrap_or_default();
                if v.trim().is_empty() { None } else { Some(v) }
            }
            _ => None,
        };

        if !GCP_AUTH_MODES.iter().any(|m| *m == auth_mode) {
            return Err(flow_like_types::anyhow!(
                "Unknown GCP auth_mode: '{}'. Expected one of {:?}",
                auth_mode,
                GCP_AUTH_MODES
            ));
        }

        let provider = GcpProvider {
            auth_mode: auth_mode.clone(),
            default_project_id: non_empty(default_project_id),
            readonly,
            service_account_json,
            service_account_file,
            access_token,
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
        "How to authenticate: 'application_default' (ADC), 'service_account_json' (raw JSON), 'service_account_file' (FlowPath), 'workload_identity' (GKE/metadata), 'access_token' (static bearer)",
        VariableType::String,
    )
    .set_options(
        PinOptions::new()
            .set_valid_values(GCP_AUTH_MODES.iter().map(|s| s.to_string()).collect())
            .build(),
    )
    .set_default_value(Some(json!(GCP_ADC)));
}

fn add_service_account_json_pin(node: &mut Node) {
    node.add_input_pin(
        "service_account_json",
        "Service Account JSON",
        "Raw JSON key contents (used when auth_mode is 'service_account_json')",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn add_service_account_file_pin(node: &mut Node) {
    node.add_input_pin(
        "service_account_file",
        "Service Account File",
        "FlowPath to the JSON key file (used when auth_mode is 'service_account_file')",
        VariableType::Struct,
    )
    .set_schema::<FlowPath>();
}

fn add_access_token_pin(node: &mut Node) {
    node.add_input_pin(
        "access_token",
        "Access Token",
        "OAuth 2.0 bearer token (used when auth_mode is 'access_token')",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn sync_auth_mode_pins(node: &mut Node, auth_mode: &str) {
    let mode = if auth_mode.is_empty() {
        GCP_ADC
    } else {
        auth_mode
    };

    let want_json = mode == GCP_SA_JSON;
    let want_file = mode == GCP_SA_FILE;
    let want_token = mode == GCP_ACCESS_TOKEN;

    let has_json = node.get_pin_by_name("service_account_json").is_some();
    let has_file = node.get_pin_by_name("service_account_file").is_some();
    let has_token = node.get_pin_by_name("access_token").is_some();

    if want_json && !has_json {
        add_service_account_json_pin(node);
    }
    if !want_json && has_json {
        remove_pin_by_name(node, "service_account_json");
    }

    if want_file && !has_file {
        add_service_account_file_pin(node);
    }
    if !want_file && has_file {
        remove_pin_by_name(node, "service_account_file");
    }

    if want_token && !has_token {
        add_access_token_pin(node);
    }
    if !want_token && has_token {
        remove_pin_by_name(node, "access_token");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;

    #[test]
    fn test_node_structure() {
        let node = GcpProviderNode::new().get_node();
        assert_eq!(node.name, "data_gcp_provider");
        assert_eq!(node.friendly_name, "GCP Provider");
        assert_eq!(node.category, "Data/Providers");
    }

    #[test]
    fn test_provider_output_is_schema_typed() {
        let node = GcpProviderNode::new().get_node();
        let out = node
            .pins
            .values()
            .find(|p| p.name == "provider" && p.pin_type == PinType::Output)
            .expect("provider output pin");
        assert_eq!(out.data_type, VariableType::Struct);
        assert!(out.schema.is_some());
    }

    #[test]
    fn test_default_shows_no_mode_specific_pins() {
        let node = GcpProviderNode::new().get_node();
        assert!(node.get_pin_by_name("service_account_json").is_none());
        assert!(node.get_pin_by_name("service_account_file").is_none());
        assert!(node.get_pin_by_name("access_token").is_none());
    }

    #[test]
    fn test_sync_auth_mode_diff_only() {
        let mut node = GcpProviderNode::new().get_node();
        sync_auth_mode_pins(&mut node, GCP_SA_JSON);
        let id_before = node
            .get_pin_by_name("service_account_json")
            .unwrap()
            .id
            .clone();
        sync_auth_mode_pins(&mut node, GCP_SA_JSON);
        let id_after = node
            .get_pin_by_name("service_account_json")
            .unwrap()
            .id
            .clone();
        assert_eq!(id_before, id_after);
    }

    #[test]
    fn test_sync_auth_mode_switch() {
        let mut node = GcpProviderNode::new().get_node();
        sync_auth_mode_pins(&mut node, GCP_SA_JSON);
        assert!(node.get_pin_by_name("service_account_json").is_some());
        sync_auth_mode_pins(&mut node, GCP_SA_FILE);
        assert!(node.get_pin_by_name("service_account_json").is_none());
        let file_pin = node.get_pin_by_name("service_account_file").unwrap();
        assert_eq!(file_pin.data_type, VariableType::Struct);
        assert!(file_pin.schema.is_some());
        sync_auth_mode_pins(&mut node, GCP_ACCESS_TOKEN);
        assert!(node.get_pin_by_name("service_account_file").is_none());
        assert!(node.get_pin_by_name("access_token").is_some());
        sync_auth_mode_pins(&mut node, GCP_ADC);
        assert!(node.get_pin_by_name("access_token").is_none());
    }
}
