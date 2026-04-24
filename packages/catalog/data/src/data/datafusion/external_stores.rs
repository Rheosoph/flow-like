use crate::data::path::FlowPath;
use crate::data::providers::aws::AwsProvider;
use crate::data::providers::azure::AzureProvider;
use crate::data::providers::cloudflare::CloudflareProvider;
use crate::data::providers::gcp::GcpProvider;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{
    aws::AmazonS3Builder, azure::MicrosoftAzureBuilder, gcp::GoogleCloudStorageBuilder,
};
use flow_like_types::{Cacheable, async_trait, json::json};
use std::sync::Arc;

// =============================================================================
// AWS S3 -> FlowPath
// =============================================================================

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
            "S3 Bucket",
            "Turn an S3 bucket (or any S3-compatible endpoint) into a FlowPath. Takes an AwsProvider for authentication. Use a CloudflareProvider + R2 node for Cloudflare R2 — it's specialised.",
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
            "provider",
            "Provider",
            "AWS provider (from the AWS Provider node)",
            VariableType::Struct,
        )
        .set_schema::<AwsProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("bucket", "Bucket", "S3 bucket name", VariableType::String);

        node.add_input_pin(
            "prefix",
            "Prefix",
            "Optional path prefix within the bucket",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "path_style",
            "Path Style",
            "Use path-style URLs (required for some S3-compatible services, e.g. MinIO)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "Done", "Store created", VariableType::Execution);
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

        let provider: AwsProvider = context.evaluate_pin("provider").await?;
        let bucket: String = context.evaluate_pin("bucket").await?;
        let prefix: String = context.evaluate_pin("prefix").await.unwrap_or_default();
        let path_style: bool = context.evaluate_pin("path_style").await.unwrap_or(false);

        if bucket.trim().is_empty() {
            return Err(flow_like_types::anyhow!("S3 bucket name is required"));
        }

        let mut builder = provider.apply_to_s3_builder(AmazonS3Builder::new());
        builder = builder.with_bucket_name(&bucket);
        if path_style {
            builder = builder.with_virtual_hosted_style_request(false);
        }

        let store = builder
            .build()
            .map_err(|e| flow_like_types::anyhow!("Failed to build S3 store: {}", e))?;
        let store = FlowLikeStore::AWS(Arc::new(store));

        let path = cache_and_wrap(
            context,
            "s3_store",
            &bucket,
            &prefix,
            &provider.auth_mode,
            store,
        )
        .await;
        context.set_pin_value("path", json!(path)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

// =============================================================================
// AWS S3 Express One Zone -> FlowPath
// =============================================================================

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
            "S3 Express Bucket",
            "Turn an S3 Express One Zone bucket into a FlowPath. Ultra-low latency single-AZ storage. Takes an AwsProvider.",
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
            "provider",
            "Provider",
            "AWS provider (from the AWS Provider node)",
            VariableType::Struct,
        )
        .set_schema::<AwsProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "bucket",
            "Bucket",
            "S3 Express bucket name (must end with --azid--x-s3)",
            VariableType::String,
        );
        node.add_input_pin(
            "prefix",
            "Prefix",
            "Optional path prefix within the bucket",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Done", "Store created", VariableType::Execution);
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

        let provider: AwsProvider = context.evaluate_pin("provider").await?;
        let bucket: String = context.evaluate_pin("bucket").await?;
        let prefix: String = context.evaluate_pin("prefix").await.unwrap_or_default();

        if bucket.trim().is_empty() {
            return Err(flow_like_types::anyhow!(
                "S3 Express bucket name is required"
            ));
        }

        let mut builder = provider.apply_to_s3_builder(AmazonS3Builder::new());
        builder = builder.with_bucket_name(&bucket).with_s3_express(true);

        let store = builder
            .build()
            .map_err(|e| flow_like_types::anyhow!("Failed to build S3 Express store: {}", e))?;
        let store = FlowLikeStore::AWS(Arc::new(store));

        let path = cache_and_wrap(
            context,
            "s3_express_store",
            &bucket,
            &prefix,
            &provider.auth_mode,
            store,
        )
        .await;
        context.set_pin_value("path", json!(path)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

// =============================================================================
// Azure Blob Container -> FlowPath
// =============================================================================

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
            "Azure Blob Container",
            "Turn an Azure Blob Storage container into a FlowPath. Takes an AzureProvider.",
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
            "provider",
            "Provider",
            "Azure provider (from the Azure Provider node)",
            VariableType::Struct,
        )
        .set_schema::<AzureProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "container",
            "Container",
            "Azure blob container name",
            VariableType::String,
        );
        node.add_input_pin(
            "prefix",
            "Prefix",
            "Optional path prefix within the container",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Done", "Store created", VariableType::Execution);
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

        let provider: AzureProvider = context.evaluate_pin("provider").await?;
        let container: String = context.evaluate_pin("container").await?;
        let prefix: String = context.evaluate_pin("prefix").await.unwrap_or_default();

        if container.trim().is_empty() {
            return Err(flow_like_types::anyhow!("Azure container name is required"));
        }

        let builder = MicrosoftAzureBuilder::new().with_container_name(&container);
        let builder = provider.apply_to_azure_builder(builder)?;

        let store = builder
            .build()
            .map_err(|e| flow_like_types::anyhow!("Failed to build Azure Blob store: {}", e))?;
        let store = FlowLikeStore::Azure(Arc::new(store));

        let path = cache_and_wrap(
            context,
            "azure_store",
            &container,
            &prefix,
            &provider.auth_mode,
            store,
        )
        .await;
        context.set_pin_value("path", json!(path)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

// =============================================================================
// GCS Bucket -> FlowPath
// =============================================================================

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
            "GCS Bucket",
            "Turn a Google Cloud Storage bucket into a FlowPath. Takes a GcpProvider.",
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
            "provider",
            "Provider",
            "GCP provider (from the GCP Provider node)",
            VariableType::Struct,
        )
        .set_schema::<GcpProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("bucket", "Bucket", "GCS bucket name", VariableType::String);
        node.add_input_pin(
            "prefix",
            "Prefix",
            "Optional path prefix within the bucket",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Done", "Store created", VariableType::Execution);
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

        let provider: GcpProvider = context.evaluate_pin("provider").await?;
        let bucket: String = context.evaluate_pin("bucket").await?;
        let prefix: String = context.evaluate_pin("prefix").await.unwrap_or_default();

        if bucket.trim().is_empty() {
            return Err(flow_like_types::anyhow!("GCS bucket name is required"));
        }

        let builder = GoogleCloudStorageBuilder::new().with_bucket_name(&bucket);
        let builder = provider.apply_to_gcs_builder(context, builder).await?;
        let builder = builder.with_bucket_name(&bucket);

        let store = builder
            .build()
            .map_err(|e| flow_like_types::anyhow!("Failed to build GCS store: {}", e))?;
        let store = FlowLikeStore::Google(Arc::new(store));

        let path = cache_and_wrap(
            context,
            "gcs_store",
            &bucket,
            &prefix,
            &provider.auth_mode,
            store,
        )
        .await;
        context.set_pin_value("path", json!(path)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

// =============================================================================
// Cloudflare R2 Bucket -> FlowPath
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CloudflareR2StoreNode {}

impl CloudflareR2StoreNode {
    pub fn new() -> Self {
        CloudflareR2StoreNode {}
    }
}

#[async_trait]
impl NodeLogic for CloudflareR2StoreNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "external_r2_store",
            "R2 Bucket",
            "Turn a Cloudflare R2 bucket into a FlowPath. Takes a CloudflareProvider in 'r2' auth mode (account_id + R2 access key/secret).",
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
            "provider",
            "Provider",
            "Cloudflare provider (from the Cloudflare Provider node, auth_mode='r2')",
            VariableType::Struct,
        )
        .set_schema::<CloudflareProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("bucket", "Bucket", "R2 bucket name", VariableType::String);
        node.add_input_pin(
            "prefix",
            "Prefix",
            "Optional path prefix within the bucket",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Done", "Store created", VariableType::Execution);
        node.add_output_pin(
            "path",
            "Path",
            "FlowPath pointing to the R2 location",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.scores = Some(NodeScores {
            privacy: 7,
            security: 7,
            performance: 8,
            governance: 7,
            reliability: 9,
            cost: 7,
        });
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let provider: CloudflareProvider = context.evaluate_pin("provider").await?;
        let bucket: String = context.evaluate_pin("bucket").await?;
        let prefix: String = context.evaluate_pin("prefix").await.unwrap_or_default();

        if bucket.trim().is_empty() {
            return Err(flow_like_types::anyhow!("R2 bucket name is required"));
        }

        let builder = AmazonS3Builder::new().with_bucket_name(&bucket);
        let builder = provider.apply_to_s3_builder_for_r2(builder)?;

        let store = builder
            .build()
            .map_err(|e| flow_like_types::anyhow!("Failed to build R2 store: {}", e))?;
        let store = FlowLikeStore::AWS(Arc::new(store));

        let auth_key = provider
            .account_id
            .as_deref()
            .unwrap_or("no_account")
            .to_string();
        let path = cache_and_wrap(context, "r2_store", &bucket, &prefix, &auth_key, store).await;
        context.set_pin_value("path", json!(path)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

// =============================================================================
// Shared cache + FlowPath wrapping
// =============================================================================

async fn cache_and_wrap(
    context: &mut ExecutionContext,
    kind: &str,
    bucket_or_container: &str,
    prefix: &str,
    auth_key: &str,
    store: FlowLikeStore,
) -> FlowPath {
    let cache_key = format!(
        "{}_{}_{}",
        kind,
        bucket_or_container,
        flow_like::utils::hash::hash_string_non_cryptographic(&format!("{}_{}", auth_key, prefix))
    );
    let cacheable: Arc<dyn Cacheable> = Arc::new(store);
    context
        .cache
        .write()
        .await
        .insert(cache_key.clone(), cacheable);
    FlowPath::new(prefix.to_string(), cache_key, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;

    fn find_pin<'a>(
        node: &'a Node,
        name: &str,
        pin_type: PinType,
    ) -> Option<&'a flow_like::flow::pin::Pin> {
        node.pins
            .values()
            .find(|p| p.name == name && p.pin_type == pin_type)
    }

    #[test]
    fn test_s3_node_takes_aws_provider() {
        let node = S3StoreNode::new().get_node();
        assert_eq!(node.name, "external_s3_store");
        let provider = find_pin(&node, "provider", PinType::Input).expect("provider input");
        assert_eq!(provider.data_type, VariableType::Struct);
        assert!(provider.schema.is_some());
    }

    #[test]
    fn test_s3_node_has_no_raw_credential_pins() {
        let node = S3StoreNode::new().get_node();
        assert!(find_pin(&node, "access_key_id", PinType::Input).is_none());
        assert!(find_pin(&node, "secret_access_key", PinType::Input).is_none());
        assert!(find_pin(&node, "session_token", PinType::Input).is_none());
        assert!(find_pin(&node, "credential_mode", PinType::Input).is_none());
        assert!(find_pin(&node, "region", PinType::Input).is_none());
        assert!(find_pin(&node, "endpoint", PinType::Input).is_none());
    }

    #[test]
    fn test_s3_express_node_takes_aws_provider() {
        let node = S3ExpressStoreNode::new().get_node();
        assert_eq!(node.name, "external_s3_express_store");
        let provider = find_pin(&node, "provider", PinType::Input).expect("provider input");
        assert_eq!(provider.data_type, VariableType::Struct);
        assert!(provider.schema.is_some());
    }

    #[test]
    fn test_azure_blob_node_takes_azure_provider() {
        let node = AzureBlobStoreNode::new().get_node();
        assert_eq!(node.name, "external_azure_blob_store");
        let provider = find_pin(&node, "provider", PinType::Input).expect("provider input");
        assert_eq!(provider.data_type, VariableType::Struct);
        assert!(provider.schema.is_some());
        // Raw credentials must not be modelled here any more.
        assert!(find_pin(&node, "access_key", PinType::Input).is_none());
        assert!(find_pin(&node, "sas_token", PinType::Input).is_none());
        assert!(find_pin(&node, "account", PinType::Input).is_none());
    }

    #[test]
    fn test_gcs_node_takes_gcp_provider() {
        let node = GcpStorageStoreNode::new().get_node();
        assert_eq!(node.name, "external_gcp_storage_store");
        let provider = find_pin(&node, "provider", PinType::Input).expect("provider input");
        assert_eq!(provider.data_type, VariableType::Struct);
        assert!(provider.schema.is_some());
        assert!(find_pin(&node, "service_account_key", PinType::Input).is_none());
    }

    #[test]
    fn test_r2_node_takes_cloudflare_provider() {
        let node = CloudflareR2StoreNode::new().get_node();
        assert_eq!(node.name, "external_r2_store");
        assert_eq!(node.friendly_name, "R2 Bucket");
        let provider = find_pin(&node, "provider", PinType::Input).expect("provider input");
        assert_eq!(provider.data_type, VariableType::Struct);
        assert!(provider.schema.is_some());
    }

    #[test]
    fn test_all_bucket_nodes_emit_flowpath() {
        for node in [
            S3StoreNode::new().get_node(),
            S3ExpressStoreNode::new().get_node(),
            AzureBlobStoreNode::new().get_node(),
            GcpStorageStoreNode::new().get_node(),
            CloudflareR2StoreNode::new().get_node(),
        ] {
            let path = find_pin(&node, "path", PinType::Output).expect("path output");
            assert_eq!(path.data_type, VariableType::Struct);
            assert!(
                path.schema.is_some(),
                "{} path output must be schema-typed FlowPath",
                node.name
            );
        }
    }
}
