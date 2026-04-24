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

pub const CF_API_TOKEN: &str = "api_token";
pub const CF_GLOBAL_API_KEY: &str = "global_api_key";
pub const CF_R2: &str = "r2";
pub const CF_ORIGIN_CA_KEY: &str = "origin_ca_key";

pub const CF_AUTH_MODES: &[&str] = &[CF_API_TOKEN, CF_GLOBAL_API_KEY, CF_R2, CF_ORIGIN_CA_KEY];

/// Typed credential struct emitted by `CloudflareProviderNode`.
///
/// Cloudflare has no single "auth" — each product (DNS API, Workers, R2) has
/// its own mechanism. This struct carries the superset so any CF-aware node
/// can pick the fields relevant to it.
#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct CloudflareProvider {
    pub auth_mode: String,
    pub account_id: Option<String>,
    // api_token mode (preferred for most APIs)
    pub api_token: Option<String>,
    // global_api_key mode (legacy)
    pub email: Option<String>,
    pub global_api_key: Option<String>,
    // r2 mode — S3-compatible access against R2. Endpoint is derived from account_id.
    pub r2_access_key_id: Option<String>,
    pub r2_secret_access_key: Option<String>,
    // origin CA key for certificate APIs
    pub origin_ca_key: Option<String>,
}

impl CloudflareProvider {
    /// Build the standard R2 S3-compatible endpoint for this provider.
    /// Returns `None` when no account_id is set.
    pub fn r2_endpoint(&self) -> Option<String> {
        self.account_id
            .as_ref()
            .filter(|id| !id.trim().is_empty())
            .map(|id| format!("https://{}.r2.cloudflarestorage.com", id))
    }

    /// Apply this provider's R2 credentials to an `AmazonS3Builder`.
    ///
    /// R2 is S3-compatible, so we reuse the AWS builder with a derived endpoint
    /// (`https://<account_id>.r2.cloudflarestorage.com`), region `"auto"` and
    /// path-style requests. Requires `auth_mode == "r2"`.
    pub fn apply_to_s3_builder_for_r2(
        &self,
        builder: flow_like_storage::object_store::aws::AmazonS3Builder,
    ) -> flow_like_types::Result<flow_like_storage::object_store::aws::AmazonS3Builder> {
        if self.auth_mode.as_str() != CF_R2 {
            return Err(flow_like_types::anyhow!(
                "CloudflareProvider: 'r2' auth_mode required (got '{}')",
                self.auth_mode
            ));
        }
        let endpoint = self.r2_endpoint().ok_or_else(|| {
            flow_like_types::anyhow!("CloudflareProvider: account_id is required for R2")
        })?;
        let key = self.r2_access_key_id.as_deref().ok_or_else(|| {
            flow_like_types::anyhow!("CloudflareProvider: r2_access_key_id is required")
        })?;
        let secret = self.r2_secret_access_key.as_deref().ok_or_else(|| {
            flow_like_types::anyhow!("CloudflareProvider: r2_secret_access_key is required")
        })?;
        Ok(builder
            .with_endpoint(endpoint)
            .with_region("auto")
            .with_access_key_id(key)
            .with_secret_access_key(secret)
            .with_virtual_hosted_style_request(false))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CloudflareProviderNode {}

impl CloudflareProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CloudflareProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_cloudflare_provider",
            "Cloudflare Provider",
            "Build a Cloudflare credential struct. Supports scoped API tokens, legacy email + global API key, R2 S3-compatible access keys and Origin CA keys. Emits a CloudflareProvider that CF-aware nodes (R2 stores, DNS API, Workers, ...) can consume.",
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
            "account_id",
            "Account ID",
            "Cloudflare account ID (required for R2 and some APIs)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        // Default auth_mode is api_token -> show its pin.
        add_api_token_pin(&mut node);

        node.add_output_pin(
            "exec_out",
            "Done",
            "Provider built",
            VariableType::Execution,
        );
        node.add_output_pin(
            "provider",
            "Provider",
            "Cloudflare provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<CloudflareProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.scores = Some(NodeScores {
            privacy: 6,
            security: 8,
            performance: 10,
            governance: 7,
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
            .unwrap_or_else(|_| CF_API_TOKEN.to_string());

        if !CF_AUTH_MODES.iter().any(|m| *m == auth_mode) {
            return Err(flow_like_types::anyhow!(
                "Unknown Cloudflare auth_mode: '{}'. Expected one of {:?}",
                auth_mode,
                CF_AUTH_MODES
            ));
        }

        let account_id = context
            .evaluate_pin::<String>("account_id")
            .await
            .ok()
            .and_then(non_empty);

        let api_token = if auth_mode.as_str() == CF_API_TOKEN {
            context
                .evaluate_pin::<String>("api_token")
                .await
                .ok()
                .and_then(non_empty)
        } else {
            None
        };

        let (email, global_api_key) = if auth_mode.as_str() == CF_GLOBAL_API_KEY {
            (
                context
                    .evaluate_pin::<String>("email")
                    .await
                    .ok()
                    .and_then(non_empty),
                context
                    .evaluate_pin::<String>("global_api_key")
                    .await
                    .ok()
                    .and_then(non_empty),
            )
        } else {
            (None, None)
        };

        let (r2_access_key_id, r2_secret_access_key) = if auth_mode.as_str() == CF_R2 {
            (
                context
                    .evaluate_pin::<String>("r2_access_key_id")
                    .await
                    .ok()
                    .and_then(non_empty),
                context
                    .evaluate_pin::<String>("r2_secret_access_key")
                    .await
                    .ok()
                    .and_then(non_empty),
            )
        } else {
            (None, None)
        };

        let origin_ca_key = if auth_mode.as_str() == CF_ORIGIN_CA_KEY {
            context
                .evaluate_pin::<String>("origin_ca_key")
                .await
                .ok()
                .and_then(non_empty)
        } else {
            None
        };

        let provider = CloudflareProvider {
            auth_mode,
            account_id,
            api_token,
            email,
            global_api_key,
            r2_access_key_id,
            r2_secret_access_key,
            origin_ca_key,
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
        "How to authenticate: 'api_token' (scoped, preferred), 'global_api_key' (legacy email+key), 'r2' (S3-compatible R2 keys), 'origin_ca_key' (Origin CA)",
        VariableType::String,
    )
    .set_options(
        PinOptions::new()
            .set_valid_values(CF_AUTH_MODES.iter().map(|s| s.to_string()).collect())
            .build(),
    )
    .set_default_value(Some(json!(CF_API_TOKEN)));
}

fn add_api_token_pin(node: &mut Node) {
    node.add_input_pin(
        "api_token",
        "API Token",
        "Scoped Cloudflare API token (dash.cloudflare.com/profile/api-tokens)",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn add_global_api_key_pins(node: &mut Node) {
    node.add_input_pin(
        "email",
        "Email",
        "Cloudflare account email (legacy global-key auth)",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "global_api_key",
        "Global API Key",
        "Legacy global API key (prefer api_token when possible)",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn remove_global_api_key_pins(node: &mut Node) {
    remove_pin_by_name(node, "email");
    remove_pin_by_name(node, "global_api_key");
}

fn add_r2_pins(node: &mut Node) {
    node.add_input_pin(
        "r2_access_key_id",
        "R2 Access Key ID",
        "R2 access key ID (S3-compatible)",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "r2_secret_access_key",
        "R2 Secret Access Key",
        "R2 secret access key (S3-compatible)",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn remove_r2_pins(node: &mut Node) {
    remove_pin_by_name(node, "r2_access_key_id");
    remove_pin_by_name(node, "r2_secret_access_key");
}

fn add_origin_ca_key_pin(node: &mut Node) {
    node.add_input_pin(
        "origin_ca_key",
        "Origin CA Key",
        "Cloudflare Origin CA key",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn sync_auth_mode_pins(node: &mut Node, auth_mode: &str) {
    let mode = if auth_mode.is_empty() {
        CF_API_TOKEN
    } else {
        auth_mode
    };

    let want_token = mode == CF_API_TOKEN;
    let want_global = mode == CF_GLOBAL_API_KEY;
    let want_r2 = mode == CF_R2;
    let want_origin = mode == CF_ORIGIN_CA_KEY;

    let has_token = node.get_pin_by_name("api_token").is_some();
    let has_global = node.get_pin_by_name("global_api_key").is_some();
    let has_r2 = node.get_pin_by_name("r2_access_key_id").is_some();
    let has_origin = node.get_pin_by_name("origin_ca_key").is_some();

    if want_token && !has_token {
        add_api_token_pin(node);
    }
    if !want_token && has_token {
        remove_pin_by_name(node, "api_token");
    }

    if want_global && !has_global {
        add_global_api_key_pins(node);
    }
    if !want_global && has_global {
        remove_global_api_key_pins(node);
    }

    if want_r2 && !has_r2 {
        add_r2_pins(node);
    }
    if !want_r2 && has_r2 {
        remove_r2_pins(node);
    }

    if want_origin && !has_origin {
        add_origin_ca_key_pin(node);
    }
    if !want_origin && has_origin {
        remove_pin_by_name(node, "origin_ca_key");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;

    #[test]
    fn test_node_structure() {
        let node = CloudflareProviderNode::new().get_node();
        assert_eq!(node.name, "data_cloudflare_provider");
        assert_eq!(node.friendly_name, "Cloudflare Provider");
        assert_eq!(node.category, "Data/Providers");
    }

    #[test]
    fn test_provider_output_is_schema_typed() {
        let node = CloudflareProviderNode::new().get_node();
        let out = node
            .pins
            .values()
            .find(|p| p.name == "provider" && p.pin_type == PinType::Output)
            .expect("provider output pin");
        assert_eq!(out.data_type, VariableType::Struct);
        assert!(out.schema.is_some());
    }

    #[test]
    fn test_default_shows_api_token_pin() {
        let node = CloudflareProviderNode::new().get_node();
        assert!(node.get_pin_by_name("api_token").is_some());
        assert!(node.get_pin_by_name("global_api_key").is_none());
        assert!(node.get_pin_by_name("r2_access_key_id").is_none());
    }

    #[test]
    fn test_sync_switches_and_is_idempotent() {
        let mut node = CloudflareProviderNode::new().get_node();
        sync_auth_mode_pins(&mut node, CF_R2);
        assert!(node.get_pin_by_name("api_token").is_none());
        assert!(node.get_pin_by_name("r2_access_key_id").is_some());
        let id_before = node.get_pin_by_name("r2_access_key_id").unwrap().id.clone();
        sync_auth_mode_pins(&mut node, CF_R2);
        let id_after = node.get_pin_by_name("r2_access_key_id").unwrap().id.clone();
        assert_eq!(id_before, id_after);
    }

    #[test]
    fn test_r2_endpoint_derivation() {
        let p = CloudflareProvider {
            account_id: Some("abcdef1234567890".to_string()),
            ..Default::default()
        };
        assert_eq!(
            p.r2_endpoint().as_deref(),
            Some("https://abcdef1234567890.r2.cloudflarestorage.com")
        );

        let empty = CloudflareProvider::default();
        assert!(empty.r2_endpoint().is_none());
    }
}
