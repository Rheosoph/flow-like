use crate::data::datafusion::session::DataFusionSession;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::sync::Arc;

/// Helper to get string value from a pin's default value
fn get_pin_string_value(node: &Node, name: &str) -> String {
    node.get_pin_by_name(name)
        .and_then(|pin| pin.default_value.clone())
        .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
        .and_then(|json| json.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// Register an AWS Athena table in DataFusion
/// This allows querying Athena data sources through DataFusion's SQL interface
#[crate::register_node]
#[derive(Default)]
pub struct RegisterAthenaNode {}

impl RegisterAthenaNode {
    pub fn new() -> Self {
        RegisterAthenaNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterAthenaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_athena",
            "Register Athena Table",
            "Register an AWS Athena table in DataFusion. Query S3 data via Athena's catalog. Supports explicit credentials or environment variables (including Lambda IAM roles).",
            "Data/DataFusion/Databases",
        );
        node.add_icon("/flow/icons/aws.svg");
        node.set_version(2);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session to register the table in",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "region",
            "Region",
            "AWS region where Athena is configured",
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
            "catalog",
            "Catalog",
            "Athena data catalog name (default: AwsDataCatalog)",
            VariableType::String,
        )
        .set_default_value(Some(json!("AwsDataCatalog")));

        node.add_input_pin(
            "athena_database",
            "Athena Database",
            "Database name in Athena",
            VariableType::String,
        );

        node.add_input_pin(
            "athena_table",
            "Athena Table",
            "Table name in Athena to query",
            VariableType::String,
        );

        node.add_input_pin(
            "output_location",
            "Output Location",
            "S3 location for query results (e.g., s3://my-bucket/athena-results/)",
            VariableType::String,
        );

        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register the table as in DataFusion",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session with registered table",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.scores = Some(NodeScores {
            privacy: 5,
            security: 6,
            performance: 6,
            governance: 7,
            reliability: 8,
            cost: 4,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let region: String = context.evaluate_pin("region").await?;
        let credential_mode: String = context
            .evaluate_pin("credential_mode")
            .await
            .unwrap_or_else(|_| "explicit".to_string());
        let access_key_id: Option<String> = context.evaluate_pin("access_key_id").await.ok();
        let secret_access_key: Option<String> =
            context.evaluate_pin("secret_access_key").await.ok();
        let session_token: Option<String> = context.evaluate_pin("session_token").await.ok();
        let catalog: String = context.evaluate_pin("catalog").await?;
        let athena_database: String = context.evaluate_pin("athena_database").await?;
        let athena_table: String = context.evaluate_pin("athena_table").await?;
        let output_location: String = context.evaluate_pin("output_location").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;

        let cached_session = session.load(context).await?;

        // Build Athena connection options based on credential mode
        let sql = match credential_mode.as_str() {
            "environment" => {
                let options = format!(
                    "'region' '{}',\n                   'output_location' '{}',\n                   'credential_mode' 'environment'",
                    region, output_location
                );
                format!(
                    r#"CREATE EXTERNAL TABLE {}
               STORED AS ATHENA
               LOCATION 'athena://{}/{}/{}'
               OPTIONS (
                   {}
               )"#,
                    table_name, catalog, athena_database, athena_table, options
                )
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

                let mut options = format!(
                    "'region' '{}',\n                   'access_key_id' '{}',\n                   'secret_access_key' '{}',\n                   'output_location' '{}'",
                    region, key, secret, output_location
                );
                if let Some(token) = &session_token
                    && !token.is_empty()
                {
                    options.push_str(&format!(
                        ",\n                   'session_token' '{}'",
                        token
                    ));
                }
                format!(
                    r#"CREATE EXTERNAL TABLE {}
               STORED AS ATHENA
               LOCATION 'athena://{}/{}/{}'
               OPTIONS (
                   {}
               )"#,
                    table_name, catalog, athena_database, athena_table, options
                )
            }
        };

        match cached_session.ctx.sql(&sql).await {
            Ok(df) => {
                df.collect().await?;
            }
            Err(e) => {
                return Err(flow_like_types::anyhow!(
                    "Athena direct integration not available. Error: {}. \
                     Alternative approaches:\n\
                     1. Use Athena to export results to S3 as Parquet, then mount the S3 path\n\
                     2. Use AWS Data Wrangler to sync Athena tables to local Parquet files\n\
                     3. Query Athena via the AWS SDK and load results as a table",
                    e
                ));
            }
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: Arc<Board>) {
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

/// Mount Athena query results from S3
/// A practical alternative that queries Athena and makes results available in DataFusion
#[crate::register_node]
#[derive(Default)]
pub struct MountAthenaQueryNode {}

impl MountAthenaQueryNode {
    pub fn new() -> Self {
        MountAthenaQueryNode {}
    }
}

#[async_trait]
impl NodeLogic for MountAthenaQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_mount_athena_query",
            "Mount Athena S3 Results",
            "Mount Parquet files from an Athena query result location in S3. Supports explicit credentials or environment variables (including Lambda IAM roles).",
            "Data/DataFusion/Databases",
        );
        node.add_icon("/flow/icons/aws.svg");
        node.set_version(2);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session to register the table in",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "s3_path",
            "S3 Path",
            "S3 path to Athena query results (e.g., s3://bucket/athena-results/query-id/)",
            VariableType::String,
        );

        node.add_input_pin("region", "Region", "AWS region", VariableType::String)
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
            "table_name",
            "Table Name",
            "Name to register the table as in DataFusion",
            VariableType::String,
        );

        node.add_input_pin(
            "format",
            "Format",
            "File format (parquet, csv)",
            VariableType::String,
        )
        .set_default_value(Some(json!("parquet")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session with registered table",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.scores = Some(NodeScores {
            privacy: 5,
            security: 6,
            performance: 8,
            governance: 7,
            reliability: 9,
            cost: 5,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::datafusion::datasource::file_format::csv::CsvFormat;
        use flow_like_storage::datafusion::datasource::file_format::parquet::ParquetFormat;
        use flow_like_storage::datafusion::datasource::listing::{
            ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl,
        };
        use flow_like_storage::object_store::aws::AmazonS3Builder;
        use flow_like_types::reqwest::Url;
        use std::sync::Arc;

        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let s3_path: String = context.evaluate_pin("s3_path").await?;
        let region: String = context.evaluate_pin("region").await?;
        let credential_mode: String = context
            .evaluate_pin("credential_mode")
            .await
            .unwrap_or_else(|_| "explicit".to_string());
        let access_key_id: Option<String> = context.evaluate_pin("access_key_id").await.ok();
        let secret_access_key: Option<String> =
            context.evaluate_pin("secret_access_key").await.ok();
        let session_token: Option<String> = context.evaluate_pin("session_token").await.ok();
        let table_name: String = context.evaluate_pin("table_name").await?;
        let format: String = context
            .evaluate_pin("format")
            .await
            .unwrap_or_else(|_| "parquet".to_string());

        let cached_session = session.load(context).await?;

        // Parse S3 URL to extract bucket
        let s3_url = Url::parse(&s3_path)?;
        let bucket = s3_url
            .host_str()
            .ok_or_else(|| flow_like_types::anyhow!("Invalid S3 URL"))?;

        // Create S3 store based on credential mode
        let s3_store = match credential_mode.as_str() {
            "environment" => {
                // Use from_env() for automatic credential discovery
                AmazonS3Builder::from_env()
                    .with_bucket_name(bucket)
                    .with_region(&region)
                    .build()?
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

                let mut builder = AmazonS3Builder::new()
                    .with_bucket_name(bucket)
                    .with_region(&region)
                    .with_access_key_id(&key)
                    .with_secret_access_key(&secret);

                if let Some(token) = &session_token
                    && !token.is_empty()
                {
                    builder = builder.with_token(token);
                }
                builder.build()?
            }
        };

        // Register the object store with the session
        let store_url = format!("s3://{}", bucket);
        cached_session
            .ctx
            .runtime_env()
            .register_object_store(&Url::parse(&store_url)?, Arc::new(s3_store));

        // Create listing table
        let table_path = ListingTableUrl::parse(&s3_path)?;

        let listing_options = match format.to_lowercase().as_str() {
            "csv" => {
                let csv_format = CsvFormat::default();
                ListingOptions::new(Arc::new(csv_format)).with_file_extension(".csv")
            }
            _ => {
                let parquet_format = ParquetFormat::default();
                ListingOptions::new(Arc::new(parquet_format)).with_file_extension(".parquet")
            }
        };

        let config = ListingTableConfig::new(table_path)
            .with_listing_options(listing_options)
            .infer_schema(&cached_session.ctx.state())
            .await?;

        let table = ListingTable::try_new(config)?;
        cached_session
            .ctx
            .register_table(&table_name, Arc::new(table))?;

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: Arc<Board>) {
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
