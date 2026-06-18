use crate::data::path::FlowPath;
use crate::data::providers::aws::AwsProvider;
use crate::data::providers::azure::AzureProvider;
use crate::data::providers::cloudflare::CloudflareProvider;
use crate::data::providers::gcp::GcpProvider;
use crate::data::providers::util::get_pin_string_value;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores, remove_pin_by_name},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::files::store::smb_store::{
    SmbAuth, SmbConfig, SmbKerberosCcacheConfig, SmbObjectStore,
};
use flow_like_storage::object_store::{
    aws::AmazonS3Builder, azure::MicrosoftAzureBuilder, gcp::GoogleCloudStorageBuilder,
};
use flow_like_types::{Cacheable, async_trait, json::json};
use std::sync::Arc;
use std::time::Duration;

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
// SMB Share -> FlowPath
// =============================================================================

const SMB_CREDENTIALS: &str = "credentials";
const SMB_GUEST: &str = "guest";
const SMB_KERBEROS_CCACHE: &str = "kerberos_ccache";
const SMB_AUTH_MODES: &[&str] = &[SMB_CREDENTIALS, SMB_GUEST, SMB_KERBEROS_CCACHE];

#[crate::register_node]
#[derive(Default)]
pub struct SmbStoreNode {}

impl SmbStoreNode {
    pub fn new() -> Self {
        SmbStoreNode {}
    }
}

#[async_trait]
impl NodeLogic for SmbStoreNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "external_smb_store",
            "SMB Share",
            "Turn an SMB2/3 share into a FlowPath.",
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
            "address",
            "Address",
            "SMB server address. Use host:port, or host to use port 445.",
            VariableType::String,
        );
        node.add_input_pin("share", "Share", "SMB share name", VariableType::String);

        add_smb_auth_mode_pin(&mut node);

        node.add_input_pin(
            "prefix",
            "Prefix",
            "Optional path prefix within the share",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        add_smb_credential_pins(&mut node);

        node.add_input_pin(
            "timeout_seconds",
            "Timeout",
            "Connection timeout in seconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5)));
        node.add_input_pin(
            "compression",
            "Compression",
            "Enable SMB compression when supported by the server",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "dfs_enabled",
            "DFS",
            "Enable DFS referral handling",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "Done", "Store created", VariableType::Execution);
        node.add_output_pin(
            "path",
            "Path",
            "FlowPath pointing to the SMB share",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.scores = Some(NodeScores {
            privacy: 5,
            security: 5,
            performance: 6,
            governance: 5,
            reliability: 6,
            cost: 8,
        });
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let address: String = context.evaluate_pin("address").await?;
        let share: String = context.evaluate_pin("share").await?;
        let auth_mode: String = context
            .evaluate_pin("auth_mode")
            .await
            .unwrap_or_else(|_| SMB_CREDENTIALS.to_string());
        let prefix: String = context.evaluate_pin("prefix").await.unwrap_or_default();
        let timeout_seconds: i64 = context.evaluate_pin("timeout_seconds").await.unwrap_or(5);
        let compression: bool = context.evaluate_pin("compression").await.unwrap_or(true);
        let dfs_enabled: bool = context.evaluate_pin("dfs_enabled").await.unwrap_or(true);

        if !SMB_AUTH_MODES.iter().any(|mode| *mode == auth_mode) {
            return Err(flow_like_types::anyhow!(
                "Unknown SMB auth_mode: '{}'. Expected one of {:?}",
                auth_mode,
                SMB_AUTH_MODES
            ));
        }

        let address = normalize_smb_address(&address)?;
        if share.trim().is_empty() {
            return Err(flow_like_types::anyhow!("SMB share name is required"));
        }

        let (username, password, domain) = if auth_mode == SMB_CREDENTIALS {
            (
                context.evaluate_pin("username").await.unwrap_or_default(),
                context.evaluate_pin("password").await.unwrap_or_default(),
                context.evaluate_pin("domain").await.unwrap_or_default(),
            )
        } else {
            (String::new(), String::new(), String::new())
        };

        let mut config = SmbConfig::new(
            address.clone(),
            share.trim().to_string(),
            username.clone(),
            password,
        );
        config.domain = domain.clone();
        config.timeout = Duration::from_secs(timeout_seconds.max(1) as u64);
        config.compression = compression;
        config.dfs_enabled = dfs_enabled;
        let auth_key = if auth_mode == SMB_KERBEROS_CCACHE {
            let kerberos_username: String = context
                .evaluate_pin("kerberos_username")
                .await
                .unwrap_or_default();
            let kerberos_realm: String = context
                .evaluate_pin("kerberos_realm")
                .await
                .unwrap_or_default();
            let kerberos_kdc_address: String = context
                .evaluate_pin("kerberos_kdc_address")
                .await
                .unwrap_or_default();
            let kerberos_ccache_path: String = context
                .evaluate_pin("kerberos_ccache_path")
                .await
                .unwrap_or_default();
            let kerberos_spn_host: String = context
                .evaluate_pin("kerberos_spn_host")
                .await
                .unwrap_or_default();

            config.auth = SmbAuth::KerberosCcache(SmbKerberosCcacheConfig {
                username: kerberos_username.clone(),
                realm: kerberos_realm.clone(),
                kdc_address: kerberos_kdc_address.clone(),
                ccache_path: kerberos_ccache_path.clone(),
                server_hostname: kerberos_spn_host.clone(),
            });
            format!(
                "{}:{}:{}:{}:{}:{}",
                auth_mode,
                kerberos_username,
                kerberos_realm,
                kerberos_kdc_address,
                kerberos_ccache_path,
                kerberos_spn_host
            )
        } else {
            format!("{}:{}:{}", auth_mode, username, domain)
        };

        let store = SmbObjectStore::connect(config)
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to connect SMB share: {}", e))?;
        let store = FlowLikeStore::Other(Arc::new(store));
        let share_key = format!("{}/{}", address, share.trim());
        let path =
            cache_and_wrap(context, "smb_store", &share_key, &prefix, &auth_key, store).await;

        context.set_pin_value("path", json!(path)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let auth_mode = get_pin_string_value(node, "auth_mode");
        sync_smb_auth_mode_pins(node, &auth_mode);
    }
}

// =============================================================================
// Shared cache + FlowPath wrapping
// =============================================================================

fn add_smb_auth_mode_pin(node: &mut Node) {
    node.add_input_pin(
        "auth_mode",
        "Auth Mode",
        "How to authenticate: 'credentials' (username/password/domain), 'guest', or 'kerberos_ccache' (local FILE ccache/kinit)",
        VariableType::String,
    )
    .set_options(
        PinOptions::new()
            .set_valid_values(SMB_AUTH_MODES.iter().map(|mode| mode.to_string()).collect())
            .build(),
    )
    .set_default_value(Some(json!(SMB_CREDENTIALS)));
}

fn add_smb_username_pin(node: &mut Node) {
    node.add_input_pin("username", "Username", "SMB username", VariableType::String)
        .set_default_value(Some(json!("")));
}

fn add_smb_password_pin(node: &mut Node) {
    node.add_input_pin("password", "Password", "SMB password", VariableType::String)
        .set_default_value(Some(json!("")))
        .set_options(PinOptions::new().set_sensitive(true).build());
}

fn add_smb_domain_pin(node: &mut Node) {
    node.add_input_pin(
        "domain",
        "Domain",
        "Optional SMB domain or workgroup",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

fn add_smb_credential_pins(node: &mut Node) {
    if node.get_pin_by_name("username").is_none() {
        add_smb_username_pin(node);
    }
    if node.get_pin_by_name("password").is_none() {
        add_smb_password_pin(node);
    }
    if node.get_pin_by_name("domain").is_none() {
        add_smb_domain_pin(node);
    }
}

fn add_smb_kerberos_username_pin(node: &mut Node) {
    node.add_input_pin(
        "kerberos_username",
        "Principal",
        "Optional Kerberos username. Defaults to the ccache principal.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

fn add_smb_kerberos_realm_pin(node: &mut Node) {
    node.add_input_pin(
        "kerberos_realm",
        "Realm",
        "Optional Kerberos realm. Defaults to the ccache principal realm.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

fn add_smb_kerberos_kdc_pin(node: &mut Node) {
    node.add_input_pin(
        "kerberos_kdc_address",
        "KDC",
        "Optional KDC address. Required if the ccache has only a TGT and needs a service ticket.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

fn add_smb_kerberos_ccache_pin(node: &mut Node) {
    node.add_input_pin(
        "kerberos_ccache_path",
        "CCache",
        "Optional FILE ccache path. Empty uses KRB5CCNAME.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

fn add_smb_kerberos_spn_pin(node: &mut Node) {
    node.add_input_pin(
        "kerberos_spn_host",
        "SPN Host",
        "Optional hostname used for the cifs/<host> service principal. Defaults to the SMB address host.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

fn add_smb_kerberos_pins(node: &mut Node) {
    if node.get_pin_by_name("kerberos_username").is_none() {
        add_smb_kerberos_username_pin(node);
    }
    if node.get_pin_by_name("kerberos_realm").is_none() {
        add_smb_kerberos_realm_pin(node);
    }
    if node.get_pin_by_name("kerberos_kdc_address").is_none() {
        add_smb_kerberos_kdc_pin(node);
    }
    if node.get_pin_by_name("kerberos_ccache_path").is_none() {
        add_smb_kerberos_ccache_pin(node);
    }
    if node.get_pin_by_name("kerberos_spn_host").is_none() {
        add_smb_kerberos_spn_pin(node);
    }
}

fn remove_smb_credential_pins(node: &mut Node) {
    remove_pin_by_name(node, "username");
    remove_pin_by_name(node, "password");
    remove_pin_by_name(node, "domain");
}

fn remove_smb_kerberos_pins(node: &mut Node) {
    remove_pin_by_name(node, "kerberos_username");
    remove_pin_by_name(node, "kerberos_realm");
    remove_pin_by_name(node, "kerberos_kdc_address");
    remove_pin_by_name(node, "kerberos_ccache_path");
    remove_pin_by_name(node, "kerberos_spn_host");
}

fn sync_smb_auth_mode_pins(node: &mut Node, auth_mode: &str) {
    let mode = if auth_mode.is_empty() {
        SMB_CREDENTIALS
    } else {
        auth_mode
    };
    let want_credentials = mode == SMB_CREDENTIALS;
    let want_kerberos = mode == SMB_KERBEROS_CCACHE;
    let has_credentials = node.get_pin_by_name("username").is_some()
        || node.get_pin_by_name("password").is_some()
        || node.get_pin_by_name("domain").is_some();
    let has_kerberos = node.get_pin_by_name("kerberos_username").is_some()
        || node.get_pin_by_name("kerberos_realm").is_some()
        || node.get_pin_by_name("kerberos_kdc_address").is_some()
        || node.get_pin_by_name("kerberos_ccache_path").is_some()
        || node.get_pin_by_name("kerberos_spn_host").is_some();

    if want_credentials {
        add_smb_credential_pins(node);
    } else if has_credentials {
        remove_smb_credential_pins(node);
    }

    if want_kerberos {
        add_smb_kerberos_pins(node);
    } else if has_kerberos {
        remove_smb_kerberos_pins(node);
    }
}

fn normalize_smb_address(address: &str) -> flow_like_types::Result<String> {
    let address = address.trim();
    if address.is_empty() {
        return Err(flow_like_types::anyhow!("SMB server address is required"));
    }

    if address.contains(':') {
        Ok(address.to_string())
    } else {
        Ok(format!("{address}:445"))
    }
}

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
    fn test_smb_node_has_connection_pins() {
        let node = SmbStoreNode::new().get_node();
        assert_eq!(node.name, "external_smb_store");
        assert_eq!(node.friendly_name, "SMB Share");
        for pin in [
            "address",
            "share",
            "prefix",
            "auth_mode",
            "username",
            "password",
            "domain",
        ] {
            let pin = find_pin(&node, pin, PinType::Input).expect("input pin");
            assert_eq!(pin.data_type, VariableType::String);
        }
        for pin in [
            "kerberos_username",
            "kerberos_realm",
            "kerberos_kdc_address",
            "kerberos_ccache_path",
            "kerberos_spn_host",
        ] {
            assert!(find_pin(&node, pin, PinType::Input).is_none());
        }
        assert_eq!(
            find_pin(&node, "timeout_seconds", PinType::Input)
                .expect("timeout input")
                .data_type,
            VariableType::Integer
        );
    }

    #[test]
    fn test_smb_auth_mode_pin_has_supported_values() {
        let node = SmbStoreNode::new().get_node();
        let pin = find_pin(&node, "auth_mode", PinType::Input).expect("auth_mode input");
        let values = pin
            .options
            .as_ref()
            .and_then(|options| options.valid_values.clone())
            .expect("auth_mode valid values");
        assert_eq!(
            values,
            vec![
                SMB_CREDENTIALS.to_string(),
                SMB_GUEST.to_string(),
                SMB_KERBEROS_CCACHE.to_string()
            ]
        );
    }

    #[test]
    fn test_smb_auth_mode_sync_switches_and_is_idempotent() {
        let mut node = SmbStoreNode::new().get_node();
        assert!(node.get_pin_by_name("username").is_some());
        let username_id = node.get_pin_by_name("username").unwrap().id.clone();

        sync_smb_auth_mode_pins(&mut node, SMB_CREDENTIALS);
        assert_eq!(
            username_id,
            node.get_pin_by_name("username").unwrap().id,
            "credentials sync should not recreate existing credential pins"
        );

        sync_smb_auth_mode_pins(&mut node, SMB_GUEST);
        assert!(node.get_pin_by_name("username").is_none());
        assert!(node.get_pin_by_name("password").is_none());
        assert!(node.get_pin_by_name("domain").is_none());
        assert!(node.get_pin_by_name("kerberos_username").is_none());

        let pin_count_after_guest = node.pins.len();
        sync_smb_auth_mode_pins(&mut node, SMB_GUEST);
        assert_eq!(
            pin_count_after_guest,
            node.pins.len(),
            "guest sync should be stable when repeated"
        );

        sync_smb_auth_mode_pins(&mut node, SMB_KERBEROS_CCACHE);
        assert!(node.get_pin_by_name("username").is_none());
        assert!(node.get_pin_by_name("password").is_none());
        assert!(node.get_pin_by_name("domain").is_none());
        assert!(node.get_pin_by_name("kerberos_username").is_some());
        assert!(node.get_pin_by_name("kerberos_realm").is_some());
        assert!(node.get_pin_by_name("kerberos_kdc_address").is_some());
        assert!(node.get_pin_by_name("kerberos_ccache_path").is_some());
        assert!(node.get_pin_by_name("kerberos_spn_host").is_some());

        let kerberos_username_id = node
            .get_pin_by_name("kerberos_username")
            .unwrap()
            .id
            .clone();
        sync_smb_auth_mode_pins(&mut node, SMB_KERBEROS_CCACHE);
        assert_eq!(
            kerberos_username_id,
            node.get_pin_by_name("kerberos_username").unwrap().id,
            "kerberos sync should not recreate existing kerberos pins"
        );

        sync_smb_auth_mode_pins(&mut node, SMB_CREDENTIALS);
        assert!(node.get_pin_by_name("username").is_some());
        assert!(node.get_pin_by_name("password").is_some());
        assert!(node.get_pin_by_name("domain").is_some());
        assert!(node.get_pin_by_name("kerberos_username").is_none());
        assert!(node.get_pin_by_name("kerberos_realm").is_none());
        assert!(node.get_pin_by_name("kerberos_kdc_address").is_none());
        assert!(node.get_pin_by_name("kerberos_ccache_path").is_none());
        assert!(node.get_pin_by_name("kerberos_spn_host").is_none());
    }

    #[test]
    fn test_all_bucket_nodes_emit_flowpath() {
        for node in [
            S3StoreNode::new().get_node(),
            S3ExpressStoreNode::new().get_node(),
            AzureBlobStoreNode::new().get_node(),
            GcpStorageStoreNode::new().get_node(),
            CloudflareR2StoreNode::new().get_node(),
            SmbStoreNode::new().get_node(),
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
