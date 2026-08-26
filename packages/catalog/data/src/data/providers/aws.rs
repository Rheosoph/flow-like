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

pub const AWS_ACCESS_KEY: &str = "access_key";
pub const AWS_ENVIRONMENT: &str = "environment";
pub const AWS_PROFILE: &str = "profile";
pub const AWS_INSTANCE_METADATA: &str = "instance_metadata";
pub const AWS_WEB_IDENTITY: &str = "web_identity";
pub const AWS_ASSUME_ROLE: &str = "assume_role";

pub const AWS_AUTH_MODES: &[&str] = &[
    AWS_ACCESS_KEY,
    AWS_ENVIRONMENT,
    AWS_PROFILE,
    AWS_INSTANCE_METADATA,
    AWS_WEB_IDENTITY,
    AWS_ASSUME_ROLE,
];

/// Typed credential struct emitted by `AwsProviderNode`.
///
/// Consumer nodes (S3, Athena, Bedrock, ...) should take a single `AwsProvider`
/// input pin instead of modeling credentials themselves.
#[derive(Default, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AwsProvider {
    pub auth_mode: String,
    pub region: String,
    /// Optional override endpoint (S3-compatible services, LocalStack, ...).
    pub endpoint_url: Option<String>,
    // access_key mode
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    // profile mode
    pub profile_name: Option<String>,
    // web_identity mode (EKS IRSA, external OIDC)
    pub web_identity_token_file: Option<String>,
    pub web_identity_role_arn: Option<String>,
    pub role_session_name: Option<String>,
    // assume_role mode (layered on top of base creds)
    pub assume_role_arn: Option<String>,
    pub assume_role_external_id: Option<String>,
    pub assume_role_session_name: Option<String>,
}

impl AwsProvider {
    /// Apply this provider's credentials and region to an `AmazonS3Builder`.
    ///
    /// Only the `access_key` mode wires credentials explicitly. All other modes
    /// rely on the AWS default credential chain (`from_env()`), which picks up:
    /// - env vars (AWS_ACCESS_KEY_ID, AWS_SESSION_TOKEN, AWS_PROFILE, ...)
    /// - shared config/credentials files
    /// - EC2 instance metadata (IMDS)
    /// - EKS web identity (AWS_WEB_IDENTITY_TOKEN_FILE + AWS_ROLE_ARN)
    /// - AssumeRole chains configured via `~/.aws/config`
    ///
    /// Callers set `bucket` on the returned builder themselves so the helper
    /// stays bucket-agnostic.
    ///
    /// Every mode except `access_key` is refused when running server-side —
    /// see [`ExecutionEnvironment::ensure_no_ambient_credentials`](flow_like::flow::execution::ExecutionEnvironment::ensure_no_ambient_credentials).
    pub fn apply_to_s3_builder(
        &self,
        context: &ExecutionContext,
        builder: flow_like_storage::object_store::aws::AmazonS3Builder,
    ) -> flow_like_types::Result<flow_like_storage::object_store::aws::AmazonS3Builder> {
        use flow_like_storage::object_store::aws::AmazonS3Builder;

        if self.relies_on_env_chain() {
            context
                .execution_environment()
                .ensure_no_ambient_credentials("AwsProvider", &self.auth_mode)?;
        }

        let mut b: AmazonS3Builder = if self.auth_mode.as_str() == AWS_ACCESS_KEY {
            let mut with_keys = builder;
            if let Some(k) = &self.access_key_id {
                with_keys = with_keys.with_access_key_id(k);
            }
            if let Some(s) = &self.secret_access_key {
                with_keys = with_keys.with_secret_access_key(s);
            }
            if let Some(t) = &self.session_token {
                with_keys = with_keys.with_token(t);
            }
            with_keys
        } else {
            // For all non-explicit modes, let the AWS default chain resolve.
            // AmazonS3Builder::from_env() picks up AWS_PROFILE, AWS_REGION, IRSA
            // env vars, IMDS, etc.
            AmazonS3Builder::from_env()
        };

        if !self.region.is_empty() {
            b = b.with_region(&self.region);
        }
        if let Some(endpoint) = &self.endpoint_url {
            b = b
                .with_endpoint(endpoint)
                .with_allow_http(endpoint.starts_with("http://"));
        }
        Ok(b)
    }

    /// Return `true` when this provider can't fully configure object_store
    /// directly and relies on process-level env vars / config files.
    ///
    /// `access_key` mode counts too when either key is missing: object_store
    /// then walks its own chain (web identity, container credentials, IMDS).
    pub fn relies_on_env_chain(&self) -> bool {
        if self.auth_mode.as_str() != AWS_ACCESS_KEY {
            return true;
        }
        let missing = |v: &Option<String>| v.as_deref().is_none_or(|s| s.trim().is_empty());
        missing(&self.access_key_id) || missing(&self.secret_access_key)
    }
}

// =============================================================================
// Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct AwsProviderNode {}

impl AwsProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AwsProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_aws_provider",
            "AWS Provider",
            "Build an AWS credential struct. Supports explicit access keys, named profiles, EC2 instance metadata, EKS web identity (IRSA), STS AssumeRole and the default environment chain. Emits an AwsProvider that any AWS-aware node (S3, Athena, Bedrock, ...) can consume.",
            "Data/Providers",
        );
        node.set_flowscript_name("data", "awsProvider");
        node.add_icon("/flow/icons/aws.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        add_auth_mode_pin(&mut node);

        node.add_input_pin(
            "region",
            "Region",
            "AWS region (e.g. 'us-east-1', 'eu-west-1')",
            VariableType::String,
        )
        .set_default_value(Some(json!("us-east-1")));

        node.add_input_pin(
            "endpoint_url",
            "Endpoint URL",
            "Override endpoint URL for S3-compatible services (LocalStack, MinIO, Cloudflare R2, ...). Leave empty for real AWS.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        // Default auth_mode is `access_key` so show those pins by default.
        add_access_key_pins(&mut node);

        node.add_output_pin(
            "exec_out",
            "Done",
            "Provider built",
            VariableType::Execution,
        );
        node.add_output_pin(
            "provider",
            "Provider",
            "AWS provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<AwsProvider>()
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
            .unwrap_or_else(|_| AWS_ACCESS_KEY.to_string());

        if !AWS_AUTH_MODES.iter().any(|m| *m == auth_mode) {
            return Err(flow_like_types::anyhow!(
                "Unknown AWS auth_mode: '{}'. Expected one of {:?}",
                auth_mode,
                AWS_AUTH_MODES
            ));
        }

        if auth_mode.as_str() != AWS_ACCESS_KEY {
            context
                .execution_environment()
                .ensure_no_ambient_credentials("AwsProvider", &auth_mode)?;
        }

        let region: String = context
            .evaluate_pin("region")
            .await
            .unwrap_or_else(|_| "us-east-1".to_string());
        let endpoint_url: Option<String> = context
            .evaluate_pin::<String>("endpoint_url")
            .await
            .ok()
            .and_then(non_empty);

        let (access_key_id, secret_access_key, session_token) =
            if auth_mode.as_str() == AWS_ACCESS_KEY {
                (
                    context
                        .evaluate_pin::<String>("access_key_id")
                        .await
                        .ok()
                        .and_then(non_empty),
                    context
                        .evaluate_pin::<String>("secret_access_key")
                        .await
                        .ok()
                        .and_then(non_empty),
                    context
                        .evaluate_pin::<String>("session_token")
                        .await
                        .ok()
                        .and_then(non_empty),
                )
            } else {
                (None, None, None)
            };

        let profile_name: Option<String> = if auth_mode.as_str() == AWS_PROFILE {
            context
                .evaluate_pin::<String>("profile_name")
                .await
                .ok()
                .and_then(non_empty)
        } else {
            None
        };

        let (web_identity_token_file, web_identity_role_arn, role_session_name) =
            if auth_mode.as_str() == AWS_WEB_IDENTITY {
                (
                    context
                        .evaluate_pin::<String>("web_identity_token_file")
                        .await
                        .ok()
                        .and_then(non_empty),
                    context
                        .evaluate_pin::<String>("web_identity_role_arn")
                        .await
                        .ok()
                        .and_then(non_empty),
                    context
                        .evaluate_pin::<String>("role_session_name")
                        .await
                        .ok()
                        .and_then(non_empty),
                )
            } else {
                (None, None, None)
            };

        let (assume_role_arn, assume_role_external_id, assume_role_session_name) =
            if auth_mode.as_str() == AWS_ASSUME_ROLE {
                (
                    context
                        .evaluate_pin::<String>("assume_role_arn")
                        .await
                        .ok()
                        .and_then(non_empty),
                    context
                        .evaluate_pin::<String>("assume_role_external_id")
                        .await
                        .ok()
                        .and_then(non_empty),
                    context
                        .evaluate_pin::<String>("assume_role_session_name")
                        .await
                        .ok()
                        .and_then(non_empty),
                )
            } else {
                (None, None, None)
            };

        let provider = AwsProvider {
            auth_mode,
            region,
            endpoint_url,
            access_key_id,
            secret_access_key,
            session_token,
            profile_name,
            web_identity_token_file,
            web_identity_role_arn,
            role_session_name,
            assume_role_arn,
            assume_role_external_id,
            assume_role_session_name,
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
        "How to authenticate: 'access_key' (static keys), 'environment' (default chain: env vars / shared config), 'profile' (~/.aws/credentials profile), 'instance_metadata' (EC2 IMDS), 'web_identity' (EKS IRSA / OIDC token file), 'assume_role' (STS AssumeRole)",
        VariableType::String,
    )
    .set_options(
        PinOptions::new()
            .set_valid_values(AWS_AUTH_MODES.iter().map(|s| s.to_string()).collect())
            .build(),
    )
    .set_default_value(Some(json!(AWS_ACCESS_KEY)));
}

fn add_access_key_pins(node: &mut Node) {
    node.add_input_pin(
        "access_key_id",
        "Access Key ID",
        "AWS access key ID (used when auth_mode is 'access_key')",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "secret_access_key",
        "Secret Access Key",
        "AWS secret access key (used when auth_mode is 'access_key')",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "session_token",
        "Session Token",
        "Optional STS session token (used when auth_mode is 'access_key')",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
}

fn remove_access_key_pins(node: &mut Node) {
    remove_pin_by_name(node, "access_key_id");
    remove_pin_by_name(node, "secret_access_key");
    remove_pin_by_name(node, "session_token");
}

fn add_profile_pin(node: &mut Node) {
    node.add_input_pin(
        "profile_name",
        "Profile Name",
        "Named profile in ~/.aws/credentials / ~/.aws/config (used when auth_mode is 'profile')",
        VariableType::String,
    )
    .set_default_value(Some(json!("default")));
}

fn add_web_identity_pins(node: &mut Node) {
    node.add_input_pin(
        "web_identity_token_file",
        "Web Identity Token File",
        "Path to the OIDC token file (e.g. EKS IRSA /var/run/secrets/eks.amazonaws.com/serviceaccount/token)",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "web_identity_role_arn",
        "Role ARN",
        "Role ARN to assume with the OIDC token",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "role_session_name",
        "Session Name",
        "STS session name",
        VariableType::String,
    )
    .set_default_value(Some(json!("flow-like")));
}

fn remove_web_identity_pins(node: &mut Node) {
    remove_pin_by_name(node, "web_identity_token_file");
    remove_pin_by_name(node, "web_identity_role_arn");
    remove_pin_by_name(node, "role_session_name");
}

fn add_assume_role_pins(node: &mut Node) {
    node.add_input_pin(
        "assume_role_arn",
        "Role ARN",
        "Target role ARN for STS AssumeRole",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "assume_role_external_id",
        "External ID",
        "Optional STS external ID",
        VariableType::String,
    )
    .set_options(PinOptions::new().set_sensitive(true).build())
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "assume_role_session_name",
        "Session Name",
        "STS session name",
        VariableType::String,
    )
    .set_default_value(Some(json!("flow-like")));
}

fn remove_assume_role_pins(node: &mut Node) {
    remove_pin_by_name(node, "assume_role_arn");
    remove_pin_by_name(node, "assume_role_external_id");
    remove_pin_by_name(node, "assume_role_session_name");
}

fn sync_auth_mode_pins(node: &mut Node, auth_mode: &str) {
    let mode = if auth_mode.is_empty() {
        AWS_ACCESS_KEY
    } else {
        auth_mode
    };

    let want_access_key = mode == AWS_ACCESS_KEY;
    let want_profile = mode == AWS_PROFILE;
    let want_web_identity = mode == AWS_WEB_IDENTITY;
    let want_assume_role = mode == AWS_ASSUME_ROLE;

    let has_access_key = node.get_pin_by_name("access_key_id").is_some();
    let has_profile = node.get_pin_by_name("profile_name").is_some();
    let has_web_identity = node.get_pin_by_name("web_identity_token_file").is_some();
    let has_assume_role = node.get_pin_by_name("assume_role_arn").is_some();

    if want_access_key && !has_access_key {
        add_access_key_pins(node);
    }
    if !want_access_key && has_access_key {
        remove_access_key_pins(node);
    }

    if want_profile && !has_profile {
        add_profile_pin(node);
    }
    if !want_profile && has_profile {
        remove_pin_by_name(node, "profile_name");
    }

    if want_web_identity && !has_web_identity {
        add_web_identity_pins(node);
    }
    if !want_web_identity && has_web_identity {
        remove_web_identity_pins(node);
    }

    if want_assume_role && !has_assume_role {
        add_assume_role_pins(node);
    }
    if !want_assume_role && has_assume_role {
        remove_assume_role_pins(node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;

    #[test]
    fn test_node_structure() {
        let node = AwsProviderNode::new().get_node();
        assert_eq!(node.name, "data_aws_provider");
        assert_eq!(node.friendly_name, "AWS Provider");
        assert_eq!(node.category, "Data/Providers");
    }

    #[test]
    fn test_provider_output_is_schema_typed() {
        let node = AwsProviderNode::new().get_node();
        let out = node
            .pins
            .values()
            .find(|p| p.name == "provider" && p.pin_type == PinType::Output)
            .expect("provider output pin");
        assert_eq!(out.data_type, VariableType::Struct);
        assert!(out.schema.is_some());
    }

    #[test]
    fn test_default_shows_access_key_pins() {
        let node = AwsProviderNode::new().get_node();
        assert!(node.get_pin_by_name("access_key_id").is_some());
        assert!(node.get_pin_by_name("secret_access_key").is_some());
        assert!(node.get_pin_by_name("session_token").is_some());
        assert!(node.get_pin_by_name("profile_name").is_none());
        assert!(node.get_pin_by_name("assume_role_arn").is_none());
    }

    #[test]
    fn test_sync_switches_and_is_idempotent() {
        let mut node = AwsProviderNode::new().get_node();

        sync_auth_mode_pins(&mut node, AWS_ASSUME_ROLE);
        assert!(node.get_pin_by_name("access_key_id").is_none());
        assert!(node.get_pin_by_name("assume_role_arn").is_some());
        let id_before = node.get_pin_by_name("assume_role_arn").unwrap().id.clone();

        sync_auth_mode_pins(&mut node, AWS_ASSUME_ROLE);
        let id_after = node.get_pin_by_name("assume_role_arn").unwrap().id.clone();
        assert_eq!(id_before, id_after);

        sync_auth_mode_pins(&mut node, AWS_ENVIRONMENT);
        assert!(node.get_pin_by_name("assume_role_arn").is_none());
        assert!(node.get_pin_by_name("access_key_id").is_none());
    }
}
