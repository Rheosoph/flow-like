use crate::data::path::FlowPath;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{
    aws::AmazonS3Builder, azure::MicrosoftAzureBuilder, gcp::GoogleCloudStorageBuilder,
};
use flow_like_types::{Cacheable, Value, async_trait, json::json};
use std::sync::Arc;

/// Helper to get string value from a pin's default value
fn get_pin_string_value(node: &Node, name: &str) -> String {
    node.get_pin_by_name(name)
        .and_then(|pin| pin.default_value.clone())
        .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
        .and_then(|json| json.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// Create an S3 store from connection details
#[crate::register_node]
#[derive(Default)]
pub struct S3StoreNode {}

impl S3StoreNode {
    pub fn new() -> Self {
        S3StoreNode {}
    }
}

#[async_trait]
impl NodeLogic for S3StoreNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "external_s3_store",
            "S3 Directory",
            "Create an S3/S3-compatible store from connection details. Works with AWS S3, MinIO, Cloudflare R2, etc. Supports explicit credentials, environment variables, or AWS profiles.",
            "Data/Files/External",
        );
        node.add_icon("/flow/icons/cloud.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin("bucket", "Bucket", "S3 bucket name", VariableType::String);

        node.add_input_pin(
            "region",
            "Region",
            "AWS region (e.g., us-east-1). Leave empty for auto-detect or S3-compatible services.",
            VariableType::String,
        )
        .set_default_value(Some(json!("us-east-1")));

        node.add_input_pin(
            "credential_mode",
            "Credential Mode",
            "How to authenticate: 'explicit' (access keys), 'environment' (env vars/Lambda IAM role/profile via AWS_PROFILE env var)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["explicit".to_string(), "environment".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("explicit")));

        node.add_input_pin(
            "access_key_id",
            "Access Key ID",
            "AWS access key ID (only used when credential_mode is 'explicit')",
            VariableType::String,
        );

        node.add_input_pin(
            "secret_access_key",
            "Secret Access Key",
            "AWS secret access key (only used when credential_mode is 'explicit')",
            VariableType::String,
        );

        node.add_input_pin(
            "session_token",
            "Session Token",
            "Optional AWS session token for temporary credentials",
            VariableType::String,
        );

        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Custom endpoint URL for S3-compatible services (MinIO, R2, etc.). Leave empty for AWS S3.",
            VariableType::String,
        );

        node.add_input_pin(
            "path_style",
            "Path Style",
            "Use path-style URLs (required for some S3-compatible services)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "prefix",
            "Prefix",
            "Optional path prefix within the bucket",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Store created successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "path",
            "Path",
            "FlowPath pointing to the S3 location",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.scores = Some(NodeScores {
            privacy: 6,
            security: 7,
            performance: 8,
            governance: 7,
            reliability: 9,
            cost: 5,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let bucket: String = context.evaluate_pin("bucket").await?;
        let region: String = context.evaluate_pin("region").await.unwrap_or_default();
        let credential_mode: String = context
            .evaluate_pin("credential_mode")
            .await
            .unwrap_or_else(|_| "explicit".to_string());
        let access_key_id: Option<String> = context.evaluate_pin("access_key_id").await.ok();
        let secret_access_key: Option<String> =
            context.evaluate_pin("secret_access_key").await.ok();
        let session_token: Option<String> = context.evaluate_pin("session_token").await.ok();
        let endpoint: Option<String> = context.evaluate_pin("endpoint").await.ok();
        let path_style: bool = context.evaluate_pin("path_style").await.unwrap_or(false);
        let prefix: String = context.evaluate_pin("prefix").await.unwrap_or_default();

        let mut builder = match credential_mode.as_str() {
            "environment" => {
                // Use from_env() to automatically pick up credentials from:
                // - Environment variables (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_SESSION_TOKEN)
                // - IAM role (when running in Lambda/EC2)
                // - AWS credentials file (~/.aws/credentials with AWS_PROFILE)

                AmazonS3Builder::from_env().with_bucket_name(&bucket)
            }
            _ => {
                // Explicit credentials mode (default)
                let key = access_key_id.ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "access_key_id is required when credential_mode is 'explicit'"
                    )
                })?;
                let secret = secret_access_key.ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "secret_access_key is required when credential_mode is 'explicit'"
                    )
                })?;

                let mut b = AmazonS3Builder::new()
                    .with_bucket_name(&bucket)
                    .with_access_key_id(&key)
                    .with_secret_access_key(&secret);

                if let Some(token) = &session_token
                    && !token.is_empty()
                {
                    b = b.with_token(token);
                }
                b
            }
        };

        if !region.is_empty() {
            builder = builder.with_region(&region);
        }

        if let Some(ep) = endpoint
            && !ep.is_empty()
        {
            builder = builder
                .with_endpoint(&ep)
                .with_allow_http(ep.starts_with("http://"));
        }

        if path_style {
            builder = builder.with_virtual_hosted_style_request(false);
        }

        let store = builder.build()?;
        let store = FlowLikeStore::AWS(Arc::new(store));

        let cache_key = format!(
            "s3_store_{}_{}",
            bucket,
            flow_like::utils::hash::hash_string_non_cryptographic(&format!(
                "{}_{}",
                credential_mode, prefix
            ))
        );

        let cacheable: Arc<dyn Cacheable> = Arc::new(store);
        context
            .cache
            .write()
            .await
            .insert(cache_key.clone(), cacheable);

        let path = FlowPath::new(prefix, cache_key, None);
        context.set_pin_value("path", json!(path)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let credential_mode = get_pin_string_value(node, "credential_mode");

        let has_access_key = node.get_pin_by_name("access_key_id").is_some();
        let has_secret_key = node.get_pin_by_name("secret_access_key").is_some();
        let has_session_token = node.get_pin_by_name("session_token").is_some();

        match credential_mode.as_str() {
            "environment" => {
                // Remove explicit credential pins when in environment mode
                if has_access_key && let Some(pin) = node.get_pin_by_name("access_key_id") {
                    node.pins.remove(&pin.id.clone());
                }
                if has_secret_key && let Some(pin) = node.get_pin_by_name("secret_access_key") {
                    node.pins.remove(&pin.id.clone());
                }
                if has_session_token && let Some(pin) = node.get_pin_by_name("session_token") {
                    node.pins.remove(&pin.id.clone());
                }
            }
            _ => {
                // Add explicit credential pins when in explicit mode (default)
                if !has_access_key {
                    node.add_input_pin(
                        "access_key_id",
                        "Access Key ID",
                        "AWS access key ID (only used when credential_mode is 'explicit')",
                        VariableType::String,
                    );
                }
                if !has_secret_key {
                    node.add_input_pin(
                        "secret_access_key",
                        "Secret Access Key",
                        "AWS secret access key (only used when credential_mode is 'explicit')",
                        VariableType::String,
                    );
                }
                if !has_session_token {
                    node.add_input_pin(
                        "session_token",
                        "Session Token",
                        "Optional AWS session token for temporary credentials",
                        VariableType::String,
                    );
                }
            }
        }
    }
}

/// Create an Azure Blob store from connection details
#[crate::register_node]
#[derive(Default)]
pub struct AzureBlobStoreNode {}

impl AzureBlobStoreNode {
    pub fn new() -> Self {
        AzureBlobStoreNode {}
    }
}

#[async_trait]
impl NodeLogic for AzureBlobStoreNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "external_azure_blob_store",
            "Azure Blob Directory",
            "Create an Azure Blob Storage store from connection details",
            "Data/Files/External",
        );
        node.add_icon("/flow/icons/cloud.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "account",
            "Account",
            "Azure storage account name",
            VariableType::String,
        );

        node.add_input_pin(
            "container",
            "Container",
            "Azure blob container name",
            VariableType::String,
        );

        node.add_input_pin(
            "access_key",
            "Access Key",
            "Azure storage access key (use this OR SAS token)",
            VariableType::String,
        );

        node.add_input_pin(
            "sas_token",
            "SAS Token",
            "Shared Access Signature token (use this OR access key)",
            VariableType::String,
        );

        node.add_input_pin(
            "prefix",
            "Prefix",
            "Optional path prefix within the container",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Store created successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "path",
            "Path",
            "FlowPath pointing to the Azure Blob location",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.scores = Some(NodeScores {
            privacy: 6,
            security: 7,
            performance: 8,
            governance: 7,
            reliability: 9,
            cost: 5,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let account: String = context.evaluate_pin("account").await?;
        let container: String = context.evaluate_pin("container").await?;
        let access_key: Option<String> = context.evaluate_pin("access_key").await.ok();
        let sas_token: Option<String> = context.evaluate_pin("sas_token").await.ok();
        let prefix: String = context.evaluate_pin("prefix").await.unwrap_or_default();

        let mut builder = MicrosoftAzureBuilder::new()
            .with_account(&account)
            .with_container_name(&container);

        if let Some(key) = access_key
            && !key.is_empty()
        {
            builder = builder.with_access_key(&key);
        }

        if let Some(sas) = sas_token
            && !sas.is_empty()
        {
            // Parse SAS token query string into key-value pairs
            let sas_str = sas.trim_start_matches('?');
            let query_pairs: Vec<(String, String)> = sas_str
                .split('&')
                .filter_map(|pair| {
                    let mut parts = pair.splitn(2, '=');
                    match (parts.next(), parts.next()) {
                        (Some(key), Some(value)) => Some((key.to_string(), value.to_string())),
                        _ => None,
                    }
                })
                .collect();
            builder = builder.with_sas_authorization(query_pairs);
        }

        let store = builder.build()?;
        let store = FlowLikeStore::Azure(Arc::new(store));

        let cache_key = format!(
            "azure_store_{}_{}",
            container,
            flow_like::utils::hash::hash_string_non_cryptographic(&format!(
                "{}{}",
                account, prefix
            ))
        );

        let cacheable: Arc<dyn Cacheable> = Arc::new(store);
        context
            .cache
            .write()
            .await
            .insert(cache_key.clone(), cacheable);

        let path = FlowPath::new(prefix, cache_key, None);
        context.set_pin_value("path", json!(path)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

/// Create a GCP Cloud Storage store from connection details
#[crate::register_node]
#[derive(Default)]
pub struct GcpStorageStoreNode {}

impl GcpStorageStoreNode {
    pub fn new() -> Self {
        GcpStorageStoreNode {}
    }
}

#[async_trait]
impl NodeLogic for GcpStorageStoreNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "external_gcp_storage_store",
            "GCP Storage Directory",
            "Create a Google Cloud Storage store from connection details",
            "Data/Files/External",
        );
        node.add_icon("/flow/icons/cloud.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin("bucket", "Bucket", "GCS bucket name", VariableType::String);

        node.add_input_pin(
            "service_account_key",
            "Service Account Key",
            "Service account JSON key (the entire JSON content)",
            VariableType::String,
        );

        node.add_input_pin(
            "prefix",
            "Prefix",
            "Optional path prefix within the bucket",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Store created successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "path",
            "Path",
            "FlowPath pointing to the GCS location",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.scores = Some(NodeScores {
            privacy: 6,
            security: 7,
            performance: 8,
            governance: 7,
            reliability: 9,
            cost: 5,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let bucket: String = context.evaluate_pin("bucket").await?;
        let service_account_key: String = context.evaluate_pin("service_account_key").await?;
        let prefix: String = context.evaluate_pin("prefix").await.unwrap_or_default();

        let builder = GoogleCloudStorageBuilder::new()
            .with_bucket_name(&bucket)
            .with_service_account_key(&service_account_key);

        let store = builder.build()?;
        let store = FlowLikeStore::Google(Arc::new(store));

        let cache_key = format!(
            "gcs_store_{}_{}",
            bucket,
            flow_like::utils::hash::hash_string_non_cryptographic(&prefix)
        );

        let cacheable: Arc<dyn Cacheable> = Arc::new(store);
        context
            .cache
            .write()
            .await
            .insert(cache_key.clone(), cacheable);

        let path = FlowPath::new(prefix, cache_key, None);
        context.set_pin_value("path", json!(path)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

/// Create an S3 Express One Zone store
#[crate::register_node]
#[derive(Default)]
pub struct S3ExpressStoreNode {}

impl S3ExpressStoreNode {
    pub fn new() -> Self {
        S3ExpressStoreNode {}
    }
}

#[async_trait]
impl NodeLogic for S3ExpressStoreNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "external_s3_express_store",
            "S3 Express Directory",
            "Create an S3 Express One Zone store for ultra-low latency access. Supports explicit credentials or environment variables (including Lambda IAM roles).",
            "Data/Files/External",
        );
        node.add_icon("/flow/icons/cloud.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "bucket",
            "Bucket",
            "S3 Express bucket name (must end with --azid--x-s3)",
            VariableType::String,
        );

        node.add_input_pin(
            "region",
            "Region",
            "AWS region where the bucket is located",
            VariableType::String,
        )
        .set_default_value(Some(json!("us-east-1")));

        node.add_input_pin(
            "credential_mode",
            "Credential Mode",
            "How to authenticate: 'explicit' (access keys), 'environment' (env vars/Lambda IAM role/profile via AWS_PROFILE env var)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["explicit".to_string(), "environment".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("explicit")));

        node.add_input_pin(
            "access_key_id",
            "Access Key ID",
            "AWS access key ID (only used when credential_mode is 'explicit')",
            VariableType::String,
        );

        node.add_input_pin(
            "secret_access_key",
            "Secret Access Key",
            "AWS secret access key (only used when credential_mode is 'explicit')",
            VariableType::String,
        );

        node.add_input_pin(
            "session_token",
            "Session Token",
            "Optional AWS session token for temporary credentials",
            VariableType::String,
        );

        node.add_input_pin(
            "prefix",
            "Prefix",
            "Optional path prefix within the bucket",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Store created successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "path",
            "Path",
            "FlowPath pointing to the S3 Express location",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.scores = Some(NodeScores {
            privacy: 6,
            security: 7,
            performance: 10,
            governance: 7,
            reliability: 9,
            cost: 4,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let bucket: String = context.evaluate_pin("bucket").await?;
        let region: String = context.evaluate_pin("region").await?;
        let credential_mode: String = context
            .evaluate_pin("credential_mode")
            .await
            .unwrap_or_else(|_| "explicit".to_string());
        let access_key_id: Option<String> = context.evaluate_pin("access_key_id").await.ok();
        let secret_access_key: Option<String> =
            context.evaluate_pin("secret_access_key").await.ok();
        let session_token: Option<String> = context.evaluate_pin("session_token").await.ok();
        let prefix: String = context.evaluate_pin("prefix").await.unwrap_or_default();

        let builder = match credential_mode.as_str() {
            "environment" => {
                // Use from_env() for automatic credential discovery
                AmazonS3Builder::from_env()
                    .with_bucket_name(&bucket)
                    .with_region(&region)
                    .with_s3_express(true)
            }
            _ => {
                let key = access_key_id.ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "access_key_id is required when credential_mode is 'explicit'"
                    )
                })?;
                let secret = secret_access_key.ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "secret_access_key is required when credential_mode is 'explicit'"
                    )
                })?;

                let mut b = AmazonS3Builder::new()
                    .with_bucket_name(&bucket)
                    .with_region(&region)
                    .with_access_key_id(&key)
                    .with_secret_access_key(&secret)
                    .with_s3_express(true);

                if let Some(token) = &session_token
                    && !token.is_empty()
                {
                    b = b.with_token(token);
                }
                b
            }
        };

        let store = builder.build()?;
        let store = FlowLikeStore::AWS(Arc::new(store));

        let cache_key = format!(
            "s3_express_store_{}_{}",
            bucket,
            flow_like::utils::hash::hash_string_non_cryptographic(&format!(
                "{}_{}",
                credential_mode, prefix
            ))
        );

        let cacheable: Arc<dyn Cacheable> = Arc::new(store);
        context
            .cache
            .write()
            .await
            .insert(cache_key.clone(), cacheable);

        let path = FlowPath::new(prefix, cache_key, None);
        context.set_pin_value("path", json!(path)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let credential_mode = get_pin_string_value(node, "credential_mode");

        let has_access_key = node.get_pin_by_name("access_key_id").is_some();
        let has_secret_key = node.get_pin_by_name("secret_access_key").is_some();
        let has_session_token = node.get_pin_by_name("session_token").is_some();

        match credential_mode.as_str() {
            "environment" => {
                if has_access_key && let Some(pin) = node.get_pin_by_name("access_key_id") {
                    node.pins.remove(&pin.id.clone());
                }
                if has_secret_key && let Some(pin) = node.get_pin_by_name("secret_access_key") {
                    node.pins.remove(&pin.id.clone());
                }
                if has_session_token && let Some(pin) = node.get_pin_by_name("session_token") {
                    node.pins.remove(&pin.id.clone());
                }
            }
            _ => {
                if !has_access_key {
                    node.add_input_pin(
                        "access_key_id",
                        "Access Key ID",
                        "AWS access key ID (only used when credential_mode is 'explicit')",
                        VariableType::String,
                    );
                }
                if !has_secret_key {
                    node.add_input_pin(
                        "secret_access_key",
                        "Secret Access Key",
                        "AWS secret access key (only used when credential_mode is 'explicit')",
                        VariableType::String,
                    );
                }
                if !has_session_token {
                    node.add_input_pin(
                        "session_token",
                        "Session Token",
                        "Optional AWS session token for temporary credentials",
                        VariableType::String,
                    );
                }
            }
        }
    }
}
