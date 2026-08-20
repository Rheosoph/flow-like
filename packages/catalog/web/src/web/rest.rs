#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like::flow::execution::{
    LogLevel, context::ExecutionContext, internal_node::InternalNode, log::LogMessage,
};
#[cfg(feature = "execute")]
use flow_like::flow::pin::{PinType, ValueType};
use flow_like::flow::{
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::{self, json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::tls::TlsConfig;

const REST_CONFIG_NODE_VERSION: u32 = 3;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct RestServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub timeout_seconds: u64,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub auth: RestAuthConfig,
    #[serde(default)]
    pub function_routes: Vec<RestFunctionRoute>,
    #[serde(default)]
    pub file_routes: Vec<RestFileRoute>,
    #[serde(default)]
    pub openapi_routes: Vec<RestOpenApiRoute>,
}

impl Default for RestServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 0,
            timeout_seconds: 0,
            max_connections: default_max_connections(),
            max_body_bytes: default_max_body_bytes(),
            tls: Default::default(),
            auth: Default::default(),
            function_routes: Vec::new(),
            file_routes: Vec::new(),
            openapi_routes: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct RestFunctionRoute {
    pub path: String,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub function_refs: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum RestRouteMethod {
    GET,
    POST,
    PUT,
    PATCH,
    ANY,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct RestFileRoute {
    pub path: String,
    pub flow_path: FlowPath,
    #[serde(default)]
    pub directory: bool,
    #[serde(default)]
    pub content_type: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct RestOpenApiRoute {
    pub path: String,
    #[serde(default)]
    pub ui_path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[derive(Default)]
pub enum RestAuthConfig {
    #[default]
    None,
    ApiKey {
        header: String,
        key: String,
    },
    BearerToken {
        token: String,
    },
    BasicAuth {
        username: String,
        password: String,
    },
    HmacSha256 {
        secret: String,
        #[serde(default = "default_hmac_signature_header")]
        signature_header: String,
        #[serde(default = "default_hmac_timestamp_header")]
        timestamp_header: String,
        #[serde(default = "default_hmac_max_skew_seconds")]
        max_skew_seconds: u64,
    },
    #[serde(rename = "oauth_bearer", alias = "o_auth_bearer")]
    OAuthBearer {
        #[serde(default)]
        issuer: Option<String>,
        #[serde(default)]
        audience: Option<String>,
        #[serde(default)]
        required_scopes: Vec<String>,
        #[serde(default)]
        jwks_url: Option<String>,
        #[serde(default)]
        jwks_flow_path: Option<FlowPath>,
        #[serde(default)]
        oidc_discovery_url: Option<String>,
    },
}

fn default_max_connections() -> u32 {
    128
}

fn default_max_body_bytes() -> usize {
    10 * 1024 * 1024
}

fn default_hmac_signature_header() -> String {
    "x-signature".to_string()
}

fn default_hmac_timestamp_header() -> String {
    "x-timestamp".to_string()
}

fn default_hmac_max_skew_seconds() -> u64 {
    300
}

fn normalize_optional_route_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(super::http_runtime::normalize_path(trimmed))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateRestServerConfigNode {}

impl CreateRestServerConfigNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateRestServerConfigNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rest_server_config",
            "REST Server Config",
            "Creates a REST server config that route, file, auth, and server nodes can compose.",
            "Web/REST",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin("host", "Host", "Bind host", VariableType::String)
            .set_default_value(Some(flow_like_types::json::json!("127.0.0.1")));
        node.add_input_pin("port", "Port", "Bind port", VariableType::Integer)
            .set_default_value(Some(flow_like_types::json::json!(0)));
        node.add_input_pin(
            "timeout_seconds",
            "Timeout Seconds",
            "Server lifetime timeout; zero means run until cancelled",
            VariableType::Integer,
        )
        .set_default_value(Some(flow_like_types::json::json!(0)));
        node.add_input_pin(
            "max_connections",
            "Max Connections",
            "Maximum concurrent requests",
            VariableType::Integer,
        )
        .set_default_value(Some(flow_like_types::json::json!(128)));
        node.add_input_pin(
            "max_body_bytes",
            "Max Body Bytes",
            "Maximum HTTP request body size",
            VariableType::Integer,
        )
        .set_default_value(Some(flow_like_types::json::json!(10485760)));
        node.add_input_pin("tls", "TLS", "TLS security config", VariableType::Struct)
            .set_schema::<TlsConfig>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "config",
            "Config",
            "REST server config",
            VariableType::Struct,
        )
        .set_schema::<RestServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let host: String = context.evaluate_pin("host").await?;
        let port: i64 = context.evaluate_pin("port").await?;
        let timeout_seconds: i64 = context.evaluate_pin("timeout_seconds").await?;
        let max_connections: i64 = context.evaluate_pin("max_connections").await?;
        let max_body_bytes: i64 = context.evaluate_pin("max_body_bytes").await?;
        let tls: TlsConfig = context.evaluate_pin("tls").await.unwrap_or_default();

        let config = RestServerConfig {
            host,
            port: port.max(0) as u16,
            timeout_seconds: timeout_seconds.max(0) as u64,
            max_connections: max_connections.max(0) as u32,
            max_body_bytes: max_body_bytes.max(0) as usize,
            tls,
            ..Default::default()
        };

        context
            .set_pin_value("config", flow_like_types::json::json!(config))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterRestFunctionNode {}

impl RegisterRestFunctionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RegisterRestFunctionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rest_register_function",
            "Register REST Function",
            "Registers referenced Flow functions as handlers for a REST path.",
            "Web/REST",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.set_can_reference_fns(true);
        node.add_input_pin(
            "config_in",
            "Config",
            "REST server config",
            VariableType::Struct,
        )
        .set_schema::<RestServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("path", "Path", "HTTP route path", VariableType::String);
        node.add_input_pin(
            "method",
            "Method",
            "Allowed HTTP method. ANY accepts all methods.",
            VariableType::String,
        )
        .set_schema::<RestRouteMethod>()
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "GET".to_string(),
                    "POST".to_string(),
                    "PUT".to_string(),
                    "PATCH".to_string(),
                    "ANY".to_string(),
                ])
                .set_enforce_schema(true)
                .build(),
        )
        .set_default_value(Some(flow_like_types::json::json!("ANY")));
        node.add_output_pin(
            "config_out",
            "Config",
            "Updated config",
            VariableType::Struct,
        )
        .set_schema::<RestServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut config: RestServerConfig = context.evaluate_pin("config_in").await?;
        let path: String = context.evaluate_pin("path").await?;
        let method: String = context
            .evaluate_pin("method")
            .await
            .unwrap_or_else(|_| "ANY".to_string());
        let refs = context
            .get_referenced_functions()
            .await?
            .into_iter()
            .map(|node| node.node_id().to_string())
            .collect();

        config.function_routes.push(RestFunctionRoute {
            path: super::http_runtime::normalize_path(&path),
            methods: rest_route_methods(&method)?,
            function_refs: refs,
        });

        context
            .set_pin_value("config_out", flow_like_types::json::json!(config))
            .await?;
        Ok(())
    }
}

fn rest_route_methods(method: &str) -> flow_like_types::Result<Vec<String>> {
    let method = method.trim().to_uppercase();
    match method.as_str() {
        "" | "ANY" => Ok(Vec::new()),
        "GET" | "POST" | "PUT" | "PATCH" => Ok(vec![method]),
        _ => Err(flow_like_types::anyhow!(
            "Invalid REST method '{}'. Expected GET, POST, PUT, PATCH, or ANY",
            method
        )),
    }
}

fn is_rest_file_directory_route(route: &RestFileRoute) -> bool {
    route.directory || rest_file_route_prefix(&route.path).is_some()
}

fn rest_file_route_prefix(path: &str) -> Option<String> {
    let path = super::http_runtime::normalize_path(path);
    let prefix = path.strip_suffix("/{filename}")?;
    Some(if prefix.is_empty() {
        "/".to_string()
    } else {
        prefix.to_string()
    })
}

fn rest_file_mount_path(path: &str) -> String {
    normalize_rest_file_mount_path(
        rest_file_route_prefix(path).unwrap_or_else(|| super::http_runtime::normalize_path(path)),
    )
}

fn normalize_rest_file_mount_path(path: String) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn rest_file_openapi_path(route: &RestFileRoute) -> String {
    if !is_rest_file_directory_route(route) {
        return super::http_runtime::normalize_path(&route.path);
    }

    let mount = rest_file_mount_path(&route.path);
    if mount == "/" {
        "/{filename}".to_string()
    } else {
        format!("{}/{{filename}}", mount.trim_end_matches('/'))
    }
}

async fn flow_path_is_directory_mount(
    flow_path: &FlowPath,
    context: &mut ExecutionContext,
) -> bool {
    let Ok(runtime) = flow_path.to_runtime(context).await else {
        return false;
    };
    runtime
        .store
        .as_generic()
        .head(&runtime.path)
        .await
        .is_err()
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterRestFilesNode {}

impl RegisterRestFilesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RegisterRestFilesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rest_register_files",
            "Register REST Files",
            "Registers a FlowPath file or directory as static REST responses.",
            "Web/REST",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "config_in",
            "Config",
            "REST server config",
            VariableType::Struct,
        )
        .set_schema::<RestServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("path", "Path", "HTTP route path", VariableType::String);
        node.add_input_pin(
            "flow_path",
            "Flow Path",
            "File or directory FlowPath",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "directory",
            "Directory",
            "Serve the FlowPath as a directory mount",
            VariableType::Boolean,
        )
        .set_default_value(Some(flow_like_types::json::json!(false)));
        node.add_input_pin(
            "content_type",
            "Content Type",
            "Optional response content type override",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("")));
        node.add_output_pin(
            "config_out",
            "Config",
            "Updated config",
            VariableType::Struct,
        )
        .set_schema::<RestServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut config: RestServerConfig = context.evaluate_pin("config_in").await?;
        let path: String = context.evaluate_pin("path").await?;
        let flow_path: FlowPath = context.evaluate_pin("flow_path").await?;
        let explicit_directory: bool = context.evaluate_pin("directory").await.unwrap_or(false);
        let content_type: String = context
            .evaluate_pin("content_type")
            .await
            .unwrap_or_default();
        let content_type = content_type.trim();
        let directory = explicit_directory
            || rest_file_route_prefix(&path).is_some()
            || flow_path_is_directory_mount(&flow_path, context).await;

        config.file_routes.push(RestFileRoute {
            path: super::http_runtime::normalize_path(&path),
            flow_path,
            directory,
            content_type: if content_type.is_empty() {
                None
            } else {
                Some(content_type.to_string())
            },
        });

        context
            .set_pin_value("config_out", flow_like_types::json::json!(config))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterRestOpenApiNode {}

impl RegisterRestOpenApiNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RegisterRestOpenApiNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rest_register_open_api",
            "Register REST OpenAPI",
            "Registers OpenAPI JSON and browser UI endpoints generated from the REST server config.",
            "Web/REST",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "config_in",
            "Config",
            "REST server config",
            VariableType::Struct,
        )
        .set_schema::<RestServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "path",
            "Path",
            "OpenAPI JSON route path",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("/openapi.json")));
        node.add_input_pin(
            "ui_path",
            "UI Path",
            "OpenAPI browser UI route path; empty disables the UI",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("/docs")));
        node.add_output_pin(
            "config_out",
            "Config",
            "Updated config",
            VariableType::Struct,
        )
        .set_schema::<RestServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut config: RestServerConfig = context.evaluate_pin("config_in").await?;
        let path: String = context
            .evaluate_pin("path")
            .await
            .unwrap_or_else(|_| "/openapi.json".to_string());
        let ui_path: String = context
            .evaluate_pin("ui_path")
            .await
            .unwrap_or_else(|_| "/docs".to_string());
        config.openapi_routes.push(RestOpenApiRoute {
            path: super::http_runtime::normalize_path(&path),
            ui_path: normalize_optional_route_path(&ui_path),
        });

        context
            .set_pin_value("config_out", flow_like_types::json::json!(config))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateApiKeyAuthNode {}

impl CreateApiKeyAuthNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateApiKeyAuthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "api_key_auth",
            "API Key Auth",
            "Creates REST auth that requires a configured API key header.",
            "Web/Auth",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "header",
            "Header",
            "Header that carries the API key",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("x-api-key")));
        node.add_input_pin("key", "Key", "Expected API key", VariableType::String);
        node.add_output_pin("auth", "Auth", "API key auth config", VariableType::Struct)
            .set_schema::<RestAuthConfig>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let header: String = context
            .evaluate_pin("header")
            .await
            .unwrap_or_else(|_| "x-api-key".to_string());
        let key: String = context.evaluate_pin("key").await?;
        context
            .set_pin_value(
                "auth",
                flow_like_types::json::json!(RestAuthConfig::ApiKey {
                    header: header.trim().to_string(),
                    key
                }),
            )
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateBearerTokenAuthNode {}

impl CreateBearerTokenAuthNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateBearerTokenAuthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "bearer_token_auth",
            "Bearer Token Auth",
            "Creates REST auth that requires a static Authorization bearer token.",
            "Web/Auth",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "token",
            "Token",
            "Expected bearer token",
            VariableType::String,
        );
        node.add_output_pin(
            "auth",
            "Auth",
            "Bearer token auth config",
            VariableType::Struct,
        )
        .set_schema::<RestAuthConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let token: String = context.evaluate_pin("token").await?;
        context
            .set_pin_value(
                "auth",
                flow_like_types::json::json!(RestAuthConfig::BearerToken { token }),
            )
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateBasicAuthNode {}

impl CreateBasicAuthNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateBasicAuthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "basic_auth",
            "Basic Auth",
            "Creates REST auth that requires HTTP Basic credentials.",
            "Web/Auth",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "username",
            "Username",
            "Expected username",
            VariableType::String,
        );
        node.add_input_pin(
            "password",
            "Password",
            "Expected password",
            VariableType::String,
        );
        node.add_output_pin("auth", "Auth", "Basic auth config", VariableType::Struct)
            .set_schema::<RestAuthConfig>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let username: String = context.evaluate_pin("username").await?;
        let password: String = context.evaluate_pin("password").await?;
        context
            .set_pin_value(
                "auth",
                flow_like_types::json::json!(RestAuthConfig::BasicAuth { username, password }),
            )
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateHmacSha256AuthNode {}

impl CreateHmacSha256AuthNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateHmacSha256AuthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "hmac_sha256_auth",
            "HMAC SHA-256 Auth",
            "Creates REST auth that verifies an HMAC-SHA256 request signature.",
            "Web/Auth",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "secret",
            "Secret",
            "Shared HMAC secret",
            VariableType::String,
        );
        node.add_input_pin(
            "signature_header",
            "Signature Header",
            "Header that carries the lowercase hex HMAC signature",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!(
            default_hmac_signature_header()
        )));
        node.add_input_pin(
            "timestamp_header",
            "Timestamp Header",
            "Header that carries the Unix timestamp in seconds",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!(
            default_hmac_timestamp_header()
        )));
        node.add_input_pin(
            "max_skew_seconds",
            "Max Skew Seconds",
            "Allowed timestamp skew in seconds; zero disables timestamp freshness checks",
            VariableType::Integer,
        )
        .set_default_value(Some(flow_like_types::json::json!(
            default_hmac_max_skew_seconds()
        )));
        node.add_output_pin("auth", "Auth", "HMAC auth config", VariableType::Struct)
            .set_schema::<RestAuthConfig>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let secret: String = context.evaluate_pin("secret").await?;
        let signature_header: String = context
            .evaluate_pin("signature_header")
            .await
            .unwrap_or_else(|_| default_hmac_signature_header());
        let timestamp_header: String = context
            .evaluate_pin("timestamp_header")
            .await
            .unwrap_or_else(|_| default_hmac_timestamp_header());
        let max_skew_seconds: i64 = context
            .evaluate_pin("max_skew_seconds")
            .await
            .unwrap_or(default_hmac_max_skew_seconds() as i64);
        context
            .set_pin_value(
                "auth",
                flow_like_types::json::json!(RestAuthConfig::HmacSha256 {
                    secret,
                    signature_header: signature_header.trim().to_string(),
                    timestamp_header: timestamp_header.trim().to_string(),
                    max_skew_seconds: max_skew_seconds.max(0) as u64,
                }),
            )
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateOidcDiscoveryAuthNode {}

impl CreateOidcDiscoveryAuthNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateOidcDiscoveryAuthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "oidc_discovery_auth",
            "OIDC Discovery Auth",
            "Creates OAuth bearer auth by discovering the JWKS URI from an OpenID Connect issuer.",
            "Web/Auth",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "issuer",
            "Issuer",
            "OIDC issuer URL. The server fetches /.well-known/openid-configuration.",
            VariableType::String,
        );
        node.add_input_pin(
            "audience",
            "Audience",
            "Required token audience. Empty disables audience validation.",
            VariableType::String,
        );
        node.add_input_pin(
            "required_scopes",
            "Required Scopes",
            "Scopes that must be present in the token scope/scp claims.",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);
        node.add_output_pin("auth", "Auth", "OIDC auth config", VariableType::Struct)
            .set_schema::<RestAuthConfig>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let issuer: String = context.evaluate_pin("issuer").await?;
        let audience: Option<String> = context.evaluate_pin("audience").await.ok();
        let required_scopes: Vec<String> = context
            .evaluate_pin("required_scopes")
            .await
            .unwrap_or_default();
        let issuer = issuer.trim().trim_end_matches('/').to_string();
        let oidc_discovery_url = format!("{}/.well-known/openid-configuration", issuer);

        context
            .set_pin_value(
                "auth",
                flow_like_types::json::json!(RestAuthConfig::OAuthBearer {
                    issuer: clean_optional(Some(issuer)),
                    audience: clean_optional(audience),
                    required_scopes,
                    jwks_url: None,
                    jwks_flow_path: None,
                    oidc_discovery_url: Some(oidc_discovery_url),
                }),
            )
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateOAuthJwksUrlAuthNode {}

impl CreateOAuthJwksUrlAuthNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateOAuthJwksUrlAuthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "oauth_jwks_url_auth",
            "OAuth JWKS URL Auth",
            "Creates OAuth bearer auth that fetches a JWKS endpoint once when the server starts.",
            "Web/Auth",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "jwks_url",
            "JWKS URL",
            "JWKS endpoint URL",
            VariableType::String,
        );
        node.add_input_pin(
            "issuer",
            "Issuer",
            "Required token issuer. Empty disables issuer validation.",
            VariableType::String,
        );
        node.add_input_pin(
            "audience",
            "Audience",
            "Required token audience. Empty disables audience validation.",
            VariableType::String,
        );
        node.add_input_pin(
            "required_scopes",
            "Required Scopes",
            "Scopes that must be present in the token scope/scp claims.",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);
        node.add_output_pin("auth", "Auth", "OAuth auth config", VariableType::Struct)
            .set_schema::<RestAuthConfig>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let jwks_url: String = context.evaluate_pin("jwks_url").await?;
        let issuer: Option<String> = context.evaluate_pin("issuer").await.ok();
        let audience: Option<String> = context.evaluate_pin("audience").await.ok();
        let required_scopes: Vec<String> = context
            .evaluate_pin("required_scopes")
            .await
            .unwrap_or_default();

        context
            .set_pin_value(
                "auth",
                flow_like_types::json::json!(RestAuthConfig::OAuthBearer {
                    issuer: clean_optional(issuer),
                    audience: clean_optional(audience),
                    required_scopes,
                    jwks_url: clean_optional(Some(jwks_url)),
                    jwks_flow_path: None,
                    oidc_discovery_url: None,
                }),
            )
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateOAuthJwksFileAuthNode {}

impl CreateOAuthJwksFileAuthNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateOAuthJwksFileAuthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "oauth_jwks_file_auth",
            "OAuth JWKS File Auth",
            "Creates OAuth bearer auth from a JWKS JSON FlowPath loaded when the server starts.",
            "Web/Auth",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "jwks_flow_path",
            "JWKS Flow Path",
            "JWKS JSON file FlowPath",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "issuer",
            "Issuer",
            "Required token issuer. Empty disables issuer validation.",
            VariableType::String,
        );
        node.add_input_pin(
            "audience",
            "Audience",
            "Required token audience. Empty disables audience validation.",
            VariableType::String,
        );
        node.add_input_pin(
            "required_scopes",
            "Required Scopes",
            "Scopes that must be present in the token scope/scp claims.",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);
        node.add_output_pin("auth", "Auth", "OAuth auth config", VariableType::Struct)
            .set_schema::<RestAuthConfig>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let jwks_flow_path: FlowPath = context.evaluate_pin("jwks_flow_path").await?;
        let issuer: Option<String> = context.evaluate_pin("issuer").await.ok();
        let audience: Option<String> = context.evaluate_pin("audience").await.ok();
        let required_scopes: Vec<String> = context
            .evaluate_pin("required_scopes")
            .await
            .unwrap_or_default();

        context
            .set_pin_value(
                "auth",
                flow_like_types::json::json!(RestAuthConfig::OAuthBearer {
                    issuer: clean_optional(issuer),
                    audience: clean_optional(audience),
                    required_scopes,
                    jwks_url: None,
                    jwks_flow_path: Some(jwks_flow_path),
                    oidc_discovery_url: None,
                }),
            )
            .await?;
        Ok(())
    }
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterRestAuthNode {}

impl RegisterRestAuthNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RegisterRestAuthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rest_register_auth",
            "Register REST Auth",
            "Registers REST server authentication settings.",
            "Web/REST",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.add_input_pin(
            "config_in",
            "Config",
            "REST server config",
            VariableType::Struct,
        )
        .set_schema::<RestServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("auth", "Auth", "Auth config", VariableType::Struct)
            .set_schema::<RestAuthConfig>()
            .set_options(PinOptions::new().set_enforce_schema(true).build())
            .set_default_value(Some(flow_like_types::json::json!({"type": "none"})));
        node.add_output_pin(
            "config_out",
            "Config",
            "Updated config",
            VariableType::Struct,
        )
        .set_schema::<RestServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut config: RestServerConfig = context.evaluate_pin("config_in").await?;
        let auth_pin = context.get_pin_by_name("auth").await?;
        let auth: RestAuthConfig = match auth_pin.depends_on().first().and_then(|pin| pin.upgrade())
        {
            Some(connected_auth) => context.evaluate_pin_ref(connected_auth).await?,
            None => RestAuthConfig::None,
        };
        config.auth = auth;
        context
            .set_pin_value("config_out", flow_like_types::json::json!(config))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RestServerNode {}

impl RestServerNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RestServerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rest_server",
            "REST Server",
            "Starts a REST server from a composed config. Function routes and files are registered on the config before this node runs.",
            "Web/REST",
        );
        node.set_version(REST_CONFIG_NODE_VERSION);
        node.add_icon("/flow/icons/web.svg");
        node.set_long_running(true);
        node.add_input_pin(
            "exec_in",
            "Execute",
            "Start server",
            VariableType::Execution,
        );
        node.add_input_pin(
            "config",
            "Config",
            "REST server config",
            VariableType::Struct,
        )
        .set_schema::<RestServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "on_listening",
            "On Listening",
            "Fires when the server is ready",
            VariableType::Execution,
        );
        node.add_output_pin(
            "local_addr",
            "Local Addr",
            "Bound address",
            VariableType::String,
        );
        node.add_output_pin(
            "on_close",
            "On Close",
            "Fires when the server stops",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Fires if the server cannot start",
            VariableType::Execution,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        };

        context.deactivate_exec_pin("on_listening").await?;
        context.deactivate_exec_pin("on_close").await?;
        context.activate_exec_pin("exec_error").await?;

        let config: RestServerConfig = context.evaluate_pin("config").await?;

        // Remote build: don't bind a socket. Emit the composed config so the
        // setup-collector on the API side can persist the registration, then
        // fire on_listening and return.
        #[cfg(all(feature = "remote", not(feature = "local")))]
        {
            if let Err(err) = super::remote::emit_remote_server_config(
                context,
                super::remote::RemoteServerKind::Rest,
                &config,
            )
            .await
            {
                context.log_message(
                    &format!("REST remote config emission failed: {}", err),
                    LogLevel::Error,
                );
                return Ok(());
            }
            context
                .set_pin_value(
                    "local_addr",
                    json!(format!("remote://rest/{}", config.host)),
                )
                .await?;
            context.deactivate_exec_pin("exec_error").await?;
            context.activate_exec_pin("on_listening").await?;
            trigger_connected_exec(context, "on_listening", "REST server (remote)").await;
            return Ok(());
        }

        #[cfg(not(all(feature = "remote", not(feature = "local"))))]
        {
            let addr = format!("{}:{}", config.host, config.port);
            let listener = match tokio::net::TcpListener::bind(&addr).await {
                Ok(listener) => listener,
                Err(err) => {
                    context.log_message(
                        &format!("REST server bind failed on {}: {}", addr, err),
                        LogLevel::Error,
                    );
                    return Ok(());
                }
            };

            let tls_acceptor = match super::tls::server_acceptor(&config.tls) {
                Ok(acceptor) => acceptor,
                Err(err) => {
                    context.log_message(
                        &format!("REST server TLS configuration failed: {}", err),
                        LogLevel::Error,
                    );
                    return Ok(());
                }
            };

            let local_addr = listener.local_addr()?.to_string();
            let function_contexts = build_function_contexts(context, &config.function_routes).await;
            let files = preload_files(context, &config.file_routes).await;
            let openapi_specs = build_openapi_specs(context, &config, &local_addr).await;
            let oauth_validator =
                match super::auth::build_oauth_validator(context, &config.auth).await {
                    Ok(validator) => validator,
                    Err(err) => {
                        context.log_message(
                            &format!("REST server OAuth configuration failed: {}", err),
                            LogLevel::Error,
                        );
                        return Ok(());
                    }
                };
            context
                .set_pin_value("local_addr", json!(local_addr))
                .await?;
            context.deactivate_exec_pin("exec_error").await?;
            context.activate_exec_pin("on_listening").await?;
            trigger_connected_exec(context, "on_listening", "REST server on_listening").await;

            let parent_node_id = context.node.node.lock().await.id.clone();
            let config = Arc::new(config);
            let function_contexts = Arc::new(function_contexts);
            let files = Arc::new(files);
            let openapi_specs = Arc::new(openapi_specs);
            let oauth_validator = Arc::new(oauth_validator);
            let cancellation_token = context.get_cancellation_token();
            let active_connections = Arc::new(AtomicU32::new(0));
            let mut handles = Vec::new();
            let mut cancelled = false;

            loop {
                let accept = if config.timeout_seconds > 0 {
                    tokio::select! {
                        result = listener.accept() => Some(result),
                        _ = tokio::time::sleep(std::time::Duration::from_secs(config.timeout_seconds)) => {
                            context.log_message("REST server timed out", LogLevel::Warn);
                            None
                        }
                        _ = super::wait_for_cancel(cancellation_token.clone()) => {
                            cancelled = true;
                            context.log_message("REST server cancelled", LogLevel::Warn);
                            None
                        }
                    }
                } else {
                    tokio::select! {
                        result = listener.accept() => Some(result),
                        _ = super::wait_for_cancel(cancellation_token.clone()) => {
                            cancelled = true;
                            context.log_message("REST server cancelled", LogLevel::Warn);
                            None
                        }
                    }
                };

                let Some(accept) = accept else {
                    break;
                };
                let (stream, remote_addr) = match accept {
                    Ok(pair) => pair,
                    Err(err) => {
                        context
                            .log_message(&format!("REST accept error: {}", err), LogLevel::Error);
                        continue;
                    }
                };

                if config.max_connections > 0
                    && active_connections.load(Ordering::Relaxed) >= config.max_connections
                {
                    context.log_message(
                        "REST server rejected request because max_connections was reached",
                        LogLevel::Warn,
                    );
                    continue;
                }

                let stream: super::tls::BoxedIo = if let Some(acceptor) = &tls_acceptor {
                    match acceptor.accept(stream).await {
                        Ok(stream) => Box::new(stream),
                        Err(err) => {
                            context.log_message(
                                &format!("REST TLS handshake failed: {}", err),
                                LogLevel::Error,
                            );
                            continue;
                        }
                    }
                } else {
                    Box::new(stream)
                };

                active_connections.fetch_add(1, Ordering::Relaxed);
                let config = config.clone();
                let function_contexts = function_contexts.clone();
                let files = files.clone();
                let openapi_specs = openapi_specs.clone();
                let oauth_validator = oauth_validator.clone();
                let active_connections = active_connections.clone();
                let parent_node_id = parent_node_id.clone();
                handles.push(tokio::spawn(async move {
                    handle_connection(
                        stream,
                        remote_addr.to_string(),
                        config,
                        function_contexts,
                        files,
                        openapi_specs,
                        oauth_validator,
                        parent_node_id,
                    )
                    .await;
                    active_connections.fetch_sub(1, Ordering::Relaxed);
                }));
            }

            for handle in handles {
                if !handle.is_finished() {
                    handle.abort();
                }
            }
            context.deactivate_exec_pin("on_listening").await?;
            context.activate_exec_pin("on_close").await?;
            trigger_connected_exec(context, "on_close", "REST server on_close").await;

            if cancelled {
                return Err(flow_like_types::anyhow!("Execution was cancelled"));
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "REST server requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
type FunctionContextMap = HashMap<String, super::http_runtime::SharedFunctionContext>;

#[cfg(feature = "execute")]
#[derive(Clone)]
enum CachedFileRoute {
    File {
        path: String,
        content_type: String,
        bytes: Vec<u8>,
    },
    Directory {
        path: String,
        prefix: flow_like_catalog_core::FlowPathRuntime,
        content_type: Option<String>,
    },
}

#[cfg(feature = "execute")]
#[derive(Clone)]
struct CachedOpenApiRoute {
    path: String,
    content_type: String,
    bytes: Vec<u8>,
}

#[cfg(feature = "execute")]
async fn build_function_contexts(
    context: &ExecutionContext,
    routes: &[RestFunctionRoute],
) -> FunctionContextMap {
    let mut map = HashMap::new();
    for route in routes {
        for id in &route.function_refs {
            if map.contains_key(id) {
                continue;
            }
            let Some(node) = context.nodes.get(id) else {
                continue;
            };
            map.insert(
                id.clone(),
                super::http_runtime::create_shared_function_context(context, node).await,
            );
        }
    }
    map
}

#[cfg(feature = "execute")]
async fn preload_files(
    context: &mut ExecutionContext,
    routes: &[RestFileRoute],
) -> Vec<CachedFileRoute> {
    let mut files = Vec::new();
    for route in routes {
        if is_rest_file_directory_route(route) {
            match route.flow_path.to_runtime(context).await {
                Ok(prefix) => files.push(CachedFileRoute::Directory {
                    path: rest_file_mount_path(&route.path),
                    prefix,
                    content_type: route.content_type.clone(),
                }),
                Err(err) => context.log_message(
                    &format!(
                        "REST file directory setup failed for {}: {}",
                        route.path, err
                    ),
                    LogLevel::Error,
                ),
            }
            continue;
        }

        let path = super::http_runtime::normalize_path(&route.path);
        match route.flow_path.get(context, false).await {
            Ok(bytes) => files.push(CachedFileRoute::File {
                path,
                content_type: route
                    .content_type
                    .clone()
                    .unwrap_or_else(|| guess_content_type(&route.path).to_string()),
                bytes,
            }),
            Err(file_err) => match route.flow_path.to_runtime(context).await {
                Ok(prefix) => files.push(CachedFileRoute::Directory {
                    path: rest_file_mount_path(&route.path),
                    prefix,
                    content_type: route.content_type.clone(),
                }),
                Err(runtime_err) => context.log_message(
                    &format!(
                        "REST file preload failed for {}: {}; directory setup failed: {}",
                        route.path, file_err, runtime_err
                    ),
                    LogLevel::Error,
                ),
            },
        }
    }
    files
}

#[cfg(feature = "execute")]
async fn build_openapi_specs(
    context: &ExecutionContext,
    config: &RestServerConfig,
    local_addr: &str,
) -> Vec<CachedOpenApiRoute> {
    if config.openapi_routes.is_empty() {
        return Vec::new();
    }

    let document = build_openapi_document(context, config, local_addr).await;
    let spec_bytes = json::to_vec_pretty(&document).unwrap_or_else(|_| b"{}".to_vec());
    let mut routes = Vec::new();

    for route in &config.openapi_routes {
        let spec_path = super::http_runtime::normalize_path(&route.path);
        routes.push(CachedOpenApiRoute {
            path: spec_path.clone(),
            content_type: "application/vnd.oai.openapi+json; charset=utf-8".to_string(),
            bytes: spec_bytes.clone(),
        });

        if let Some(ui_path) = route
            .ui_path
            .as_deref()
            .and_then(normalize_optional_route_path)
            && ui_path != spec_path
        {
            routes.push(CachedOpenApiRoute {
                path: ui_path,
                content_type: "text/html; charset=utf-8".to_string(),
                bytes: openapi_ui_html(&spec_path).into_bytes(),
            });
        }
    }

    routes
}

#[cfg(feature = "execute")]
async fn build_openapi_document(
    context: &ExecutionContext,
    config: &RestServerConfig,
    local_addr: &str,
) -> flow_like_types::Value {
    let mut paths = json::Map::new();
    let board_refs = context
        .get_board()
        .await
        .map(|board| board.refs.clone())
        .unwrap_or_default();

    for file in &config.file_routes {
        let path = rest_file_openapi_path(file);
        let content_type = file
            .content_type
            .clone()
            .unwrap_or_else(|| guess_content_type(&path).to_string());
        let mut operation = json!({
            "operationId": openapi_operation_id("get", &path),
            "summary": if is_rest_file_directory_route(file) {
                "Static directory file"
            } else {
                "Static file"
            },
            "responses": openapi_responses(&config.auth, Some((&content_type, response_schema_for_content_type(&content_type))))
        });
        if is_rest_file_directory_route(file) {
            operation["parameters"] = json!([{
                "name": "filename",
                "in": "path",
                "required": true,
                "schema": {"type": "string"}
            }]);
        }
        insert_openapi_operation(&mut paths, path.clone(), "get", operation);
    }

    for route in &config.function_routes {
        let path = super::http_runtime::normalize_path(&route.path);
        let request_schema =
            route_request_body_schema(context, &route.function_refs, &board_refs).await;
        let (summary, description, function_refs) =
            route_function_metadata(context, &route.function_refs, &board_refs).await;

        for method in openapi_methods(&route.methods) {
            let mut operation = json!({
                "operationId": openapi_operation_id(method, &path),
                "summary": summary.clone().unwrap_or_else(|| "REST function route".to_string()),
                "responses": openapi_responses(&config.auth, Some(("application/json", json!({
                    "type": "object",
                    "additionalProperties": true
                })))),
                "x-flow-like-functions": function_refs
            });
            if let Some(description) = &description {
                operation["description"] = json!(description);
            }
            if method != "get" && method != "head" {
                operation["requestBody"] = json!({
                    "required": false,
                    "content": {
                        "application/json": {
                            "schema": request_schema.clone()
                        }
                    }
                });
            }
            insert_openapi_operation(&mut paths, path.clone(), method, operation);
        }
    }

    let mut document = json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Flow Like REST Server",
            "version": env!("CARGO_PKG_VERSION")
        },
        "servers": [{
            "url": format!("{}://{}", if config.tls.secure { "https" } else { "http" }, local_addr)
        }],
        "paths": paths
    });

    if let Some((name, scheme, requirement)) = openapi_security(&config.auth) {
        let mut security_schemes = json::Map::new();
        security_schemes.insert(name, scheme);
        document["components"] = json!({
            "securitySchemes": security_schemes
        });
        document["security"] = json!([requirement]);
    }

    document
}

#[cfg(feature = "execute")]
fn openapi_ui_html(spec_path: &str) -> String {
    let spec_url = json::to_string(spec_path).unwrap_or_else(|_| "\"/openapi.json\"".to_string());
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Flow Like REST API</title>
  <link rel="icon" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/favicon-32x32.png">
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css">
  <style>
    html, body {{
      margin: 0;
      min-height: 100%;
      background: #ffffff;
    }}
    #swagger-ui .topbar {{
      background-color: #111827;
    }}
    #swagger-ui .topbar .download-url-wrapper .select-label {{
      color: #e5e7eb;
    }}
  </style>
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js" crossorigin></script>
  <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-standalone-preset.js" crossorigin></script>
  <script>
    window.addEventListener("load", function () {{
      window.ui = SwaggerUIBundle({{
        url: {spec_url},
        dom_id: "#swagger-ui",
        deepLinking: true,
        displayRequestDuration: true,
        filter: true,
        persistAuthorization: true,
        showCommonExtensions: true,
        showExtensions: true,
        tryItOutEnabled: true,
        presets: [
          SwaggerUIBundle.presets.apis,
          SwaggerUIStandalonePreset
        ],
        plugins: [
          SwaggerUIBundle.plugins.DownloadUrl
        ],
        layout: "StandaloneLayout"
      }});
    }});
  </script>
</body>
</html>"##
    )
}

#[cfg(feature = "execute")]
fn insert_openapi_operation(
    paths: &mut json::Map<String, flow_like_types::Value>,
    path: String,
    method: &str,
    operation: flow_like_types::Value,
) {
    if !paths.contains_key(&path) {
        paths.insert(path.clone(), json!({}));
    }
    if let Some(path_item) = paths.get_mut(&path).and_then(|value| value.as_object_mut()) {
        path_item.entry(method.to_string()).or_insert(operation);
    }
}

#[cfg(feature = "execute")]
fn openapi_methods(methods: &[String]) -> Vec<&'static str> {
    let allowed = [
        ("GET", "get"),
        ("POST", "post"),
        ("PUT", "put"),
        ("PATCH", "patch"),
        ("DELETE", "delete"),
        ("OPTIONS", "options"),
        ("HEAD", "head"),
        ("TRACE", "trace"),
    ];
    if methods.is_empty() {
        return allowed
            .iter()
            .filter_map(|(upper, lower)| (*upper != "TRACE").then_some(*lower))
            .collect();
    }

    let mut out = Vec::new();
    for method in methods {
        let upper = method.to_uppercase();
        if let Some((_, lower)) = allowed.iter().find(|(name, _)| *name == upper)
            && !out.contains(lower)
        {
            out.push(*lower);
        }
    }
    out
}

#[cfg(feature = "execute")]
async fn route_function_metadata(
    context: &ExecutionContext,
    function_refs: &[String],
    board_refs: &HashMap<String, String>,
) -> (Option<String>, Option<String>, Vec<String>) {
    let mut names = Vec::new();
    let mut descriptions = Vec::new();
    let mut refs = Vec::new();

    for id in function_refs {
        let Some(node) = context.nodes.get(id) else {
            continue;
        };
        let guard = node.node.lock().await;
        refs.push(id.clone());
        if !guard.friendly_name.is_empty() {
            names.push(guard.friendly_name.clone());
        }
        if let Some(description) = resolved_openapi_description(&guard.description, board_refs) {
            descriptions.push(description);
        }
    }

    let summary = if names.is_empty() {
        None
    } else {
        Some(names.join(", "))
    };
    let description = if descriptions.is_empty() {
        None
    } else {
        Some(descriptions.join("\n\n"))
    };

    (summary, description, refs)
}

#[cfg(feature = "execute")]
async fn route_request_body_schema(
    context: &ExecutionContext,
    function_refs: &[String],
    board_refs: &HashMap<String, String>,
) -> flow_like_types::Value {
    let mut payload_schemas = Vec::new();
    let mut properties = json::Map::new();
    let mut has_named_body_properties = false;

    for id in function_refs {
        let Some(node) = context.nodes.get(id) else {
            continue;
        };
        let guard = node.node.lock().await;
        for pin in guard.pins.values() {
            if pin.pin_type != PinType::Output || pin.data_type == VariableType::Execution {
                continue;
            }
            let resolved_schema = resolve_openapi_text_ref_opt(pin.schema.as_deref(), board_refs);
            if pin.name == "payload" {
                payload_schemas.push(openapi_pin_schema(
                    &pin.data_type,
                    &pin.value_type,
                    resolved_schema.as_deref(),
                    &pin.description,
                    board_refs,
                ));
                continue;
            }
            if is_rest_internal_arg_pin(&pin.name) {
                continue;
            }
            has_named_body_properties = true;
            let property_name = openapi_pin_property_name(pin);
            properties.insert(
                property_name,
                openapi_pin_schema(
                    &pin.data_type,
                    &pin.value_type,
                    resolved_schema.as_deref(),
                    &pin.description,
                    board_refs,
                ),
            );
        }
    }

    if has_named_body_properties {
        let mut schema = json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": true
        });
        if !payload_schemas.is_empty() {
            schema["x-flow-like-payload-schema"] = merge_openapi_schemas(payload_schemas);
        }
        return schema;
    }

    if !payload_schemas.is_empty() {
        return merge_openapi_schemas(payload_schemas);
    }

    if properties.is_empty() {
        json!({
            "type": "object",
            "additionalProperties": true
        })
    } else {
        json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": true
        })
    }
}

#[cfg(feature = "execute")]
fn is_rest_internal_arg_pin(name: &str) -> bool {
    matches!(
        name,
        "_client"
            | "request"
            | "method"
            | "path"
            | "query"
            | "headers"
            | "body"
            | "body_text"
            | "body_bytes"
    )
}

#[cfg(feature = "execute")]
fn merge_openapi_schemas(mut schemas: Vec<flow_like_types::Value>) -> flow_like_types::Value {
    if schemas.len() == 1 {
        schemas.remove(0)
    } else {
        json!({ "allOf": schemas })
    }
}

#[cfg(feature = "execute")]
fn openapi_pin_schema(
    data_type: &VariableType,
    value_type: &ValueType,
    schema: Option<&str>,
    description: &str,
    board_refs: &HashMap<String, String>,
) -> flow_like_types::Value {
    let mut base = match data_type {
        VariableType::String | VariableType::PathBuf | VariableType::Date => {
            json!({"type": "string"})
        }
        VariableType::Integer | VariableType::Byte => json!({"type": "integer"}),
        VariableType::Float => json!({"type": "number"}),
        VariableType::Boolean => json!({"type": "boolean"}),
        VariableType::Struct | VariableType::Generic => schema
            .and_then(|schema| json::from_str::<flow_like_types::Value>(schema).ok())
            .unwrap_or_else(|| json!({"type": "object", "additionalProperties": true})),
        VariableType::Execution => json!({"type": "null"}),
    };
    if let Some(obj) = base.as_object_mut()
        && let Some(description) = resolved_openapi_description(description, board_refs)
    {
        obj.insert("description".to_string(), json!(description));
    }
    match value_type {
        ValueType::Array | ValueType::HashSet => json!({"type": "array", "items": base}),
        ValueType::HashMap => json!({"type": "object", "additionalProperties": base}),
        ValueType::Normal => base,
    }
}

#[cfg(feature = "execute")]
fn openapi_pin_property_name(pin: &flow_like::flow::pin::Pin) -> String {
    let friendly = super::http_runtime::sanitize_identifier(pin.friendly_name.trim());
    if !friendly.is_empty() {
        return friendly;
    }
    super::http_runtime::sanitize_identifier(&pin.name)
}

#[cfg(feature = "execute")]
const OPENAPI_EMPTY_STRING_HASH: &str = "16248035215404677707";

#[cfg(feature = "execute")]
fn resolve_openapi_text_ref(value: &str, board_refs: &HashMap<String, String>) -> String {
    let trimmed = value.trim();
    if trimmed == OPENAPI_EMPTY_STRING_HASH {
        return String::new();
    }
    board_refs
        .get(trimmed)
        .cloned()
        .unwrap_or_else(|| trimmed.to_string())
}

#[cfg(feature = "execute")]
fn resolve_openapi_text_ref_opt(
    value: Option<&str>,
    board_refs: &HashMap<String, String>,
) -> Option<String> {
    value.map(|raw| resolve_openapi_text_ref(raw, board_refs))
}

#[cfg(feature = "execute")]
fn resolved_openapi_description(
    description: &str,
    board_refs: &HashMap<String, String>,
) -> Option<String> {
    let resolved = resolve_openapi_text_ref(description, board_refs);
    let trimmed = resolved.trim();
    if trimmed.is_empty() || trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(feature = "execute")]
fn response_schema_for_content_type(content_type: &str) -> flow_like_types::Value {
    let lower = content_type.to_ascii_lowercase();
    if lower.contains("json") {
        json!({"type": "object", "additionalProperties": true})
    } else if lower.starts_with("text/") || lower.contains("xml") || lower.contains("html") {
        json!({"type": "string"})
    } else {
        json!({"type": "string", "format": "binary"})
    }
}

#[cfg(feature = "execute")]
fn openapi_responses(
    auth: &RestAuthConfig,
    ok_content: Option<(&str, flow_like_types::Value)>,
) -> flow_like_types::Value {
    let mut responses = json::Map::new();
    let ok = if let Some((content_type, schema)) = ok_content {
        let mut content = json::Map::new();
        content.insert(
            content_type.to_string(),
            json!({
                "schema": schema
            }),
        );
        json!({
            "description": "Successful response",
            "content": content
        })
    } else {
        json!({"description": "Successful response"})
    };
    responses.insert("200".to_string(), ok);

    if !matches!(auth, RestAuthConfig::None) {
        responses.insert(
            "401".to_string(),
            json!({"description": "Authentication required or token invalid"}),
        );
        if matches!(auth, RestAuthConfig::OAuthBearer { .. }) {
            responses.insert(
                "403".to_string(),
                json!({"description": "Authenticated token is missing required scope"}),
            );
        }
    }

    flow_like_types::Value::Object(responses)
}

#[cfg(feature = "execute")]
fn openapi_security(
    auth: &RestAuthConfig,
) -> Option<(String, flow_like_types::Value, flow_like_types::Value)> {
    match auth {
        RestAuthConfig::None => None,
        RestAuthConfig::ApiKey { header, .. } => Some((
            "ApiKeyAuth".to_string(),
            json!({
                "type": "apiKey",
                "in": "header",
                "name": header
            }),
            json!({"ApiKeyAuth": []}),
        )),
        RestAuthConfig::BearerToken { .. } => Some((
            "BearerAuth".to_string(),
            json!({
                "type": "http",
                "scheme": "bearer"
            }),
            json!({"BearerAuth": []}),
        )),
        RestAuthConfig::BasicAuth { .. } => Some((
            "BasicAuth".to_string(),
            json!({
                "type": "http",
                "scheme": "basic"
            }),
            json!({"BasicAuth": []}),
        )),
        RestAuthConfig::HmacSha256 {
            signature_header,
            timestamp_header,
            max_skew_seconds,
            ..
        } => Some((
            "HmacSha256Auth".to_string(),
            json!({
                "type": "apiKey",
                "in": "header",
                "name": signature_header,
                "x-timestamp-header": timestamp_header,
                "x-max-skew-seconds": max_skew_seconds,
                "x-signature-algorithm": "HMAC-SHA256",
                "x-signature-canonical-string": "METHOD\\nPATH\\nTIMESTAMP\\nBODY_SHA256_HEX"
            }),
            json!({"HmacSha256Auth": []}),
        )),
        RestAuthConfig::OAuthBearer {
            issuer,
            audience,
            required_scopes,
            jwks_url,
            jwks_flow_path,
            oidc_discovery_url,
        } => {
            let mut scheme = json!({
                "type": "http",
                "scheme": "bearer",
                "bearerFormat": "JWT",
                "x-required-scopes": required_scopes
            });
            if let Some(issuer) = issuer {
                scheme["x-issuer"] = json!(issuer);
            }
            if let Some(audience) = audience {
                scheme["x-audience"] = json!(audience);
            }
            if let Some(jwks_url) = jwks_url {
                scheme["x-jwks-url"] = json!(jwks_url);
            } else if jwks_flow_path.is_some() {
                scheme["x-jwks-source"] = json!("flow_path");
            } else if let Some(oidc_discovery_url) = oidc_discovery_url {
                scheme["x-oidc-discovery-url"] = json!(oidc_discovery_url);
            }
            Some((
                "OAuthBearer".to_string(),
                scheme,
                json!({"OAuthBearer": []}),
            ))
        }
    }
}

#[cfg(feature = "execute")]
fn openapi_operation_id(method: &str, path: &str) -> String {
    let path = super::http_runtime::sanitize_identifier(path).replace('-', "_");
    if path.is_empty() {
        method.to_string()
    } else {
        format!("{}_{}", method, path)
    }
}

#[cfg(feature = "execute")]
#[allow(clippy::too_many_arguments)]
async fn handle_connection(
    mut stream: super::tls::BoxedIo,
    remote_addr: String,
    config: std::sync::Arc<RestServerConfig>,
    function_contexts: std::sync::Arc<FunctionContextMap>,
    files: std::sync::Arc<Vec<CachedFileRoute>>,
    openapi_specs: std::sync::Arc<Vec<CachedOpenApiRoute>>,
    oauth_validator: std::sync::Arc<Option<super::auth::OAuthValidator>>,
    parent_node_id: String,
) {
    let response = match super::http_runtime::read_http_request(
        &mut *stream,
        remote_addr,
        config.max_body_bytes,
    )
    .await
    {
        Ok(Some(request)) => {
            route_request(
                request,
                &config,
                &function_contexts,
                &files,
                &openapi_specs,
                oauth_validator.as_ref().as_ref(),
                &parent_node_id,
            )
            .await
        }
        Ok(None) => return,
        Err(err) => super::http_runtime::HttpResponse::text(400, format!("Bad request: {}", err)),
    };

    if let Err(err) = super::http_runtime::write_http_response(&mut *stream, response).await {
        tracing::warn!("REST response write failed: {}", err);
    }
}

#[cfg(feature = "execute")]
async fn route_request(
    request: super::http_runtime::HttpRequest,
    config: &RestServerConfig,
    function_contexts: &FunctionContextMap,
    files: &[CachedFileRoute],
    openapi_specs: &[CachedOpenApiRoute],
    oauth_validator: Option<&super::auth::OAuthValidator>,
    parent_node_id: &str,
) -> super::http_runtime::HttpResponse {
    let client =
        match super::auth::authorize_client(&config.auth, oauth_validator, &request, "rest") {
            Ok(client) => client,
            Err(response) => return response,
        };

    if let Some(spec) = openapi_specs.iter().find(|spec| spec.path == request.path) {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), spec.content_type.clone());
        return super::http_runtime::HttpResponse {
            status_code: 200,
            headers,
            body: spec.bytes.clone(),
        };
    }

    for file in files {
        if let Some(response) = file_route_response(file, &request.path).await {
            return response;
        }
    }

    let Some(route) = config
        .function_routes
        .iter()
        .find(|route| route.path == request.path && method_matches(route, &request.method))
    else {
        return super::http_runtime::HttpResponse::text(404, "Not Found");
    };

    let body = match parse_rest_body_value(&request) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let mut result = json!({"ok": true});
    let args = rest_arguments(&request, &client, body);
    for function_id in &route.function_refs {
        let Some(ctx) = function_contexts.get(function_id) else {
            return super::http_runtime::HttpResponse::text(
                500,
                format!("Registered function not found: {}", function_id),
            );
        };
        match super::http_runtime::trigger_shared_function_context(
            ctx,
            &args,
            parent_node_id,
            "REST route handler",
        )
        .await
        {
            Ok(value) => result = value,
            Err(err) => {
                return super::http_runtime::HttpResponse::json(
                    500,
                    json!({"error": err.to_string()}),
                );
            }
        }
    }

    response_from_value(result)
}

#[cfg(feature = "execute")]
async fn file_route_response(
    file: &CachedFileRoute,
    request_path: &str,
) -> Option<super::http_runtime::HttpResponse> {
    let request_path = super::http_runtime::normalize_path(request_path);
    match file {
        CachedFileRoute::File {
            path,
            content_type,
            bytes,
        } if path == &request_path => Some(file_response(content_type.clone(), bytes.clone())),
        CachedFileRoute::Directory {
            path,
            prefix,
            content_type,
        } => {
            let filename = rest_file_route_filename(path, &request_path)?;
            let decoded_filename = decode_rest_file_name(&filename);
            let object_path = prefix.path.child(decoded_filename.as_str());
            let file = match prefix.store.as_generic().get(&object_path).await {
                Ok(file) => file,
                Err(_) => {
                    return Some(super::http_runtime::HttpResponse::text(404, "Not Found"));
                }
            };
            let bytes = match file.bytes().await {
                Ok(bytes) => bytes.to_vec(),
                Err(_) => {
                    return Some(super::http_runtime::HttpResponse::text(404, "Not Found"));
                }
            };
            let content_type = content_type
                .clone()
                .unwrap_or_else(|| guess_content_type(&decoded_filename).to_string());
            Some(file_response(content_type, bytes))
        }
        _ => None,
    }
}

#[cfg(feature = "execute")]
fn file_response(content_type: String, body: Vec<u8>) -> super::http_runtime::HttpResponse {
    let mut headers = HashMap::new();
    headers.insert("content-type".to_string(), content_type);
    super::http_runtime::HttpResponse {
        status_code: 200,
        headers,
        body,
    }
}

fn rest_file_route_filename(route_path: &str, request_path: &str) -> Option<String> {
    let prefix = rest_file_mount_path(route_path);
    let request_path = super::http_runtime::normalize_path(request_path);
    let filename = if prefix == "/" {
        request_path.strip_prefix('/')?
    } else {
        request_path.strip_prefix(&format!("{}/", prefix))?
    };

    if filename.is_empty() || filename.contains('/') {
        return None;
    }

    Some(filename.to_string())
}

#[cfg(feature = "execute")]
fn decode_rest_file_name(filename: &str) -> String {
    urlencoding::decode(filename)
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| filename.to_string())
}

#[cfg(feature = "execute")]
fn rest_arguments(
    request: &super::http_runtime::HttpRequest,
    client: &flow_like_types::Value,
    body: flow_like_types::Value,
) -> flow_like_types::Value {
    let mut args = rest_args_from_body_and_query(&body, &request.query);
    let payload = super::auth::payload_with_client(body.clone(), client);
    args.insert("payload".to_string(), payload);
    let request_value = rest_request_to_value_with_client(request, client, &body);
    args.insert("request".to_string(), request_value);
    args.insert("method".to_string(), json!(request.method));
    args.insert("path".to_string(), json!(request.path));
    args.insert("query".to_string(), json!(request.query));
    args.insert("headers".to_string(), json!(request.headers));
    args.insert("body".to_string(), body);
    args.insert(
        "body_text".to_string(),
        json!(String::from_utf8(request.body.clone()).ok()),
    );
    args.insert("body_bytes".to_string(), json!(request.body));
    args.insert("_client".to_string(), client.clone());
    flow_like_types::Value::Object(args)
}

#[cfg(feature = "execute")]
fn rest_args_from_body_and_query(
    body: &flow_like_types::Value,
    query: &HashMap<String, String>,
) -> json::Map<String, flow_like_types::Value> {
    let mut args = json::Map::new();

    for (key, value) in query {
        if !is_rest_internal_arg_pin(key) {
            args.insert(key.clone(), flow_like_types::Value::String(value.clone()));
        }
    }

    if let Some(body_object) = body.as_object() {
        for (key, value) in body_object {
            args.insert(key.clone(), value.clone());
        }
    }

    args
}

#[cfg(feature = "execute")]
fn parse_rest_body_value(
    request: &super::http_runtime::HttpRequest,
) -> Result<flow_like_types::Value, super::http_runtime::HttpResponse> {
    let content_type = request
        .headers
        .get("content-type")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();

    if request.body.is_empty() || !content_type.contains("application/json") {
        return Ok(super::http_runtime::parse_body_value(request));
    }

    json::from_slice::<flow_like_types::Value>(&request.body).map_err(|err| {
        super::http_runtime::HttpResponse::json(
            400,
            json!({
                "error": format!("Invalid JSON request body: {}", err)
            }),
        )
    })
}

#[cfg(feature = "execute")]
fn rest_request_to_value_with_client(
    request: &super::http_runtime::HttpRequest,
    client: &flow_like_types::Value,
    body: &flow_like_types::Value,
) -> flow_like_types::Value {
    let body_text = String::from_utf8(request.body.clone()).ok();
    json!({
        "method": request.method,
        "path": request.path,
        "query": request.query,
        "headers": request.headers,
        "body": body,
        "body_text": body_text,
        "body_bytes": request.body,
        "_client": client
    })
}

#[cfg(feature = "execute")]
fn response_from_value(value: flow_like_types::Value) -> super::http_runtime::HttpResponse {
    if let Some(obj) = value.as_object() {
        let status_code = obj
            .get("status_code")
            .or_else(|| obj.get("status"))
            .and_then(|value| value.as_u64())
            .map(|status| status as u16)
            .unwrap_or(200);
        let headers = obj
            .get("headers")
            .and_then(|value| value.as_object())
            .map(|headers| {
                headers
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .as_str()
                            .map(|value| (name.to_lowercase(), value.to_string()))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();

        if obj.contains_key("body") {
            return body_response(
                status_code,
                headers,
                obj.get("body").cloned().unwrap_or_default(),
            );
        }
    }

    body_response(200, HashMap::new(), value)
}

#[cfg(feature = "execute")]
fn body_response(
    status_code: u16,
    mut headers: HashMap<String, String>,
    body: flow_like_types::Value,
) -> super::http_runtime::HttpResponse {
    let bytes = match body {
        flow_like_types::Value::Null => Vec::new(),
        flow_like_types::Value::String(text) => {
            headers
                .entry("content-type".to_string())
                .or_insert_with(|| "text/plain; charset=utf-8".to_string());
            text.into_bytes()
        }
        other => {
            headers
                .entry("content-type".to_string())
                .or_insert_with(|| "application/json; charset=utf-8".to_string());
            json::to_vec(&other).unwrap_or_default()
        }
    };

    super::http_runtime::HttpResponse {
        status_code,
        headers,
        body: bytes,
    }
}

#[cfg(feature = "execute")]
fn method_matches(route: &RestFunctionRoute, method: &str) -> bool {
    route.methods.is_empty()
        || route
            .methods
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(method))
}

pub(crate) fn guess_content_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(feature = "execute")]
async fn trigger_connected_exec(context: &mut ExecutionContext, pin_name: &str, log_name: &str) {
    let Ok(pin) = context.get_pin_by_name(pin_name).await else {
        return;
    };

    for node in pin.get_connected_nodes() {
        let mut sub = context.create_sub_context(&node).await;
        sub.delegated = true;
        let mut message = LogMessage::new(log_name, LogLevel::Debug, None);
        let _ = InternalNode::trigger(&mut sub, &mut None, true).await;
        message.end();
        sub.log(message);
        sub.end_trace();
        context.push_sub_context(&mut sub);
    }
}

#[cfg(test)]
mod serialization_tests {
    use super::{RestAuthConfig, RestFileRoute, RestServerConfig};
    use flow_like_catalog_core::FlowPath;
    use flow_like_types::json::json;

    #[test]
    fn oauth_bearer_auth_uses_canonical_type_and_accepts_legacy_alias() {
        let auth = RestAuthConfig::OAuthBearer {
            issuer: None,
            audience: None,
            required_scopes: Vec::new(),
            jwks_url: None,
            jwks_flow_path: None,
            oidc_discovery_url: None,
        };

        assert_eq!(
            flow_like_types::json::to_value(&auth).unwrap()["type"],
            json!("oauth_bearer")
        );

        let legacy = flow_like_types::json::from_value::<RestAuthConfig>(json!({
            "type": "o_auth_bearer"
        }))
        .unwrap();

        assert!(matches!(legacy, RestAuthConfig::OAuthBearer { .. }));
    }

    #[test]
    fn rest_server_config_serializes_file_routes() {
        let config = RestServerConfig {
            file_routes: vec![RestFileRoute {
                path: "/assets".to_string(),
                flow_path: FlowPath::new(
                    "storage/assets".to_string(),
                    "dirs__storage_test".to_string(),
                    None,
                ),
                directory: true,
                content_type: Some("text/plain".to_string()),
            }],
            ..Default::default()
        };

        let value = flow_like_types::json::to_value(config).unwrap();
        assert_eq!(value["file_routes"][0]["path"], json!("/assets"));
        assert_eq!(value["file_routes"][0]["directory"], json!(true));
        assert_eq!(value["file_routes"][0]["content_type"], json!("text/plain"));
        assert_eq!(
            value["file_routes"][0]["flow_path"]["path"],
            json!("storage/assets")
        );
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::web::test_support::{
        internal_node, internal_node_with_logic, output_value, test_context,
    };
    use flow_like::flow::node::NodeLogic;
    use flow_like_storage::{
        Path,
        files::store::FlowLikeStore,
        object_store::{ObjectStore, PutPayload, memory::InMemory},
    };
    use flow_like_types::{Cacheable, Value, async_trait, json::json};
    use std::sync::Arc;

    #[derive(Default)]
    struct RestEchoLogic;

    #[async_trait]
    impl NodeLogic for RestEchoLogic {
        fn get_node(&self) -> Node {
            Node::new("rest_echo", "REST Echo", "REST echo test", "Tests")
        }

        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            let path: String = context.evaluate_pin("path").await?;
            let payload: Value = context.evaluate_pin("payload").await?;
            let client: Value = context.evaluate_pin("_client").await?;
            context.set_result(json!({
                "status_code": 201,
                "body": {
                    "path": path,
                    "payload": payload,
                    "client": client
                }
            }));
            Ok(())
        }
    }

    #[derive(Default)]
    struct RestQueryLogic;

    #[async_trait]
    impl NodeLogic for RestQueryLogic {
        fn get_node(&self) -> Node {
            Node::new(
                "rest_query_echo",
                "REST Query Echo",
                "REST query echo test",
                "Tests",
            )
        }

        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            let name: String = context.evaluate_pin("name").await?;
            let payload: Value = context.evaluate_pin("payload").await?;
            let query: Value = context.evaluate_pin("query").await?;
            let request: Value = context.evaluate_pin("request").await?;
            context.set_result(json!({
                "body": {
                    "name": name,
                    "payload": payload,
                    "query": query,
                    "request_query": request.get("query").cloned().unwrap_or(Value::Null)
                }
            }));
            Ok(())
        }
    }

    fn rest_handler() -> Arc<InternalNode> {
        let mut node = Node::new(
            "test_rest_handler",
            "Test REST Handler",
            "Handler test node",
            "Tests",
        );
        node.add_output_pin("path", "Path", "Path", VariableType::String);
        node.add_output_pin("payload", "Payload", "Payload", VariableType::Struct);
        node.add_output_pin("_client", "Client", "Client", VariableType::Struct);
        internal_node_with_logic(node, Arc::new(RestEchoLogic))
    }

    fn rest_query_handler() -> Arc<InternalNode> {
        let mut node = Node::new(
            "test_rest_query_handler",
            "Test REST Query Handler",
            "Handler query test node",
            "Tests",
        );
        node.add_output_pin("name", "Name", "Name", VariableType::String);
        node.add_output_pin("payload", "Payload", "Payload", VariableType::Struct);
        node.add_output_pin("query", "Query", "Query", VariableType::Struct);
        node.add_output_pin("request", "Request", "Request", VariableType::Struct);
        internal_node_with_logic(node, Arc::new(RestQueryLogic))
    }

    #[tokio::test]
    async fn rest_register_files_node_writes_file_route_fields() {
        let node = internal_node(RegisterRestFilesNode::new().get_node());
        let mut context = test_context(node, vec![]).await;
        let flow_path = FlowPath::new(
            "storage/assets".to_string(),
            "dirs__storage_test".to_string(),
            None,
        );

        context
            .set_pin_value("config_in", json!(RestServerConfig::default()))
            .await
            .unwrap();
        context
            .set_pin_value("path", json!("/assets"))
            .await
            .unwrap();
        context
            .set_pin_value("flow_path", json!(flow_path))
            .await
            .unwrap();
        context
            .set_pin_value("directory", json!(true))
            .await
            .unwrap();
        context
            .set_pin_value("content_type", json!("text/plain"))
            .await
            .unwrap();

        RegisterRestFilesNode::new()
            .run(&mut context)
            .await
            .unwrap();

        let config = output_value(&context, "config_out").await.unwrap();
        assert_eq!(config["file_routes"][0]["path"], json!("/assets"));
        assert_eq!(config["file_routes"][0]["directory"], json!(true));
        assert_eq!(
            config["file_routes"][0]["content_type"],
            json!("text/plain")
        );
        assert_eq!(
            config["file_routes"][0]["flow_path"]["path"],
            json!("storage/assets")
        );
    }

    #[tokio::test]
    async fn rest_register_auth_node_clears_disconnected_stale_auth() {
        let node = internal_node(RegisterRestAuthNode::new().get_node());
        let mut context = test_context(node, vec![]).await;

        context
            .set_pin_value("config_in", json!(RestServerConfig::default()))
            .await
            .unwrap();
        context
            .set_pin_value(
                "auth",
                json!({
                    "type": "api_key",
                    "header": "x-api-key",
                    "key": "stale"
                }),
            )
            .await
            .unwrap();

        RegisterRestAuthNode::new().run(&mut context).await.unwrap();

        let config = output_value(&context, "config_out").await.unwrap();
        assert_eq!(config["auth"]["type"], json!("none"));
    }

    #[tokio::test]
    async fn rest_route_calls_registered_function_and_overwrites_client_metadata() {
        let handler = rest_handler();
        let parent = internal_node(RestServerNode::new().get_node());
        let context = test_context(parent, vec![handler.clone()]).await;
        let mut functions = FunctionContextMap::new();
        functions.insert(
            handler.node_id().to_string(),
            super::super::http_runtime::create_shared_function_context(&context, &handler).await,
        );
        let config = RestServerConfig {
            function_routes: vec![RestFunctionRoute {
                path: "/echo".to_string(),
                methods: vec!["POST".to_string()],
                function_refs: vec![handler.node_id().to_string()],
            }],
            ..Default::default()
        };
        let request = super::super::http_runtime::HttpRequest {
            method: "POST".to_string(),
            path: "/echo".to_string(),
            query: HashMap::new(),
            headers: HashMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: flow_like_types::json::to_vec(&json!({
                "_client": "attacker",
                "payload": {"message": "hi"}
            }))
            .unwrap(),
            remote_addr: "127.0.0.1:1234".to_string(),
        };

        let response = route_request(request, &config, &functions, &[], &[], None, "parent").await;
        assert_eq!(response.status_code, 201);
        let body: Value = flow_like_types::json::from_slice(&response.body).unwrap();
        assert_eq!(body["path"], json!("/echo"));
        assert_eq!(body["payload"]["payload"], json!({"message": "hi"}));
        assert_eq!(body["payload"]["_client"]["protocol"], json!("rest"));
        assert_eq!(body["client"]["protocol"], json!("rest"));
        assert_eq!(body["client"]["remote_addr"], json!("127.0.0.1:1234"));
    }

    #[tokio::test]
    async fn rest_route_uses_query_params_as_named_args_with_body_precedence() {
        let handler = rest_query_handler();
        let parent = internal_node(RestServerNode::new().get_node());
        let context = test_context(parent, vec![handler.clone()]).await;
        let mut functions = FunctionContextMap::new();
        functions.insert(
            handler.node_id().to_string(),
            super::super::http_runtime::create_shared_function_context(&context, &handler).await,
        );
        let config = RestServerConfig {
            function_routes: vec![RestFunctionRoute {
                path: "/search".to_string(),
                methods: vec!["GET".to_string(), "POST".to_string()],
                function_refs: vec![handler.node_id().to_string()],
            }],
            ..Default::default()
        };

        let get_request = super::super::http_runtime::HttpRequest {
            method: "GET".to_string(),
            path: "/search".to_string(),
            query: HashMap::from([
                ("name".to_string(), "from-query".to_string()),
                ("payload".to_string(), "ignored".to_string()),
            ]),
            headers: HashMap::new(),
            body: Vec::new(),
            remote_addr: "127.0.0.1:1234".to_string(),
        };
        let response =
            route_request(get_request, &config, &functions, &[], &[], None, "parent").await;
        assert_eq!(response.status_code, 200);
        let body: Value = flow_like_types::json::from_slice(&response.body).unwrap();
        assert_eq!(body["name"], json!("from-query"));
        assert_eq!(body["query"]["name"], json!("from-query"));
        assert_eq!(body["request_query"]["name"], json!("from-query"));
        assert_eq!(body["payload"]["_client"]["protocol"], json!("rest"));
        assert_ne!(body["payload"], json!("ignored"));

        let post_request = super::super::http_runtime::HttpRequest {
            method: "POST".to_string(),
            path: "/search".to_string(),
            query: HashMap::from([("name".to_string(), "from-query".to_string())]),
            headers: HashMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: flow_like_types::json::to_vec(&json!({
                "name": "from-body"
            }))
            .unwrap(),
            remote_addr: "127.0.0.1:1234".to_string(),
        };
        let response =
            route_request(post_request, &config, &functions, &[], &[], None, "parent").await;
        assert_eq!(response.status_code, 200);
        let body: Value = flow_like_types::json::from_slice(&response.body).unwrap();
        assert_eq!(body["name"], json!("from-body"));
        assert_eq!(body["query"]["name"], json!("from-query"));
        assert_eq!(body["request_query"]["name"], json!("from-query"));
    }

    #[tokio::test]
    async fn rest_auth_blocks_request_before_route_resolution() {
        let config = RestServerConfig {
            auth: RestAuthConfig::ApiKey {
                header: "x-api-key".to_string(),
                key: "secret".to_string(),
            },
            ..Default::default()
        };
        let request = super::super::http_runtime::HttpRequest {
            method: "GET".to_string(),
            path: "/missing".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: Vec::new(),
            remote_addr: "127.0.0.1:1234".to_string(),
        };

        let response =
            route_request(request, &config, &HashMap::new(), &[], &[], None, "parent").await;
        assert_eq!(response.status_code, 401);
    }

    #[tokio::test]
    async fn rest_route_rejects_malformed_json_before_calling_function() {
        let config = RestServerConfig {
            function_routes: vec![RestFunctionRoute {
                path: "/form".to_string(),
                methods: vec!["POST".to_string()],
                function_refs: vec!["handler".to_string()],
            }],
            ..Default::default()
        };
        let request = super::super::http_runtime::HttpRequest {
            method: "POST".to_string(),
            path: "/form".to_string(),
            query: HashMap::new(),
            headers: HashMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: br#"{"name":"string","age":0,}"#.to_vec(),
            remote_addr: "127.0.0.1:1234".to_string(),
        };

        let response =
            route_request(request, &config, &HashMap::new(), &[], &[], None, "parent").await;

        assert_eq!(response.status_code, 400);
        let body: Value = flow_like_types::json::from_slice(&response.body).unwrap();
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("Invalid JSON request body")
        );
    }

    #[tokio::test]
    async fn rest_directory_file_route_serves_requested_filename() {
        let parent = internal_node(RestServerNode::new().get_node());
        let mut context = test_context(parent, vec![]).await;
        let memory = Arc::new(InMemory::new());
        memory
            .put(
                &Path::from("assets").child("hello.txt"),
                PutPayload::from("hello"),
            )
            .await
            .unwrap();
        memory
            .put(
                &Path::from("assets").child("nested/name.txt"),
                PutPayload::from("encoded slash"),
            )
            .await
            .unwrap();
        let store: Arc<dyn Cacheable> = Arc::new(FlowLikeStore::Memory(memory));
        context.set_cache("dirs__upload_test", store).await;

        let flow_path = FlowPath::new("assets".to_string(), "dirs__upload_test".to_string(), None);
        let directory = flow_path_is_directory_mount(&flow_path, &mut context).await;
        assert!(directory);

        let config = RestServerConfig {
            file_routes: vec![RestFileRoute {
                path: "/files".to_string(),
                flow_path,
                directory,
                content_type: None,
            }],
            ..Default::default()
        };
        let files = preload_files(&mut context, &config.file_routes).await;
        let spec = build_openapi_document(&context, &config, "127.0.0.1:8080").await;
        assert!(spec["paths"]["/files/{filename}"]["get"].is_object());
        assert_eq!(
            rest_file_route_filename("/files/{filename}", "/files/hello.txt"),
            Some("hello.txt".to_string())
        );
        let request = super::super::http_runtime::HttpRequest {
            method: "GET".to_string(),
            path: "/files/hello.txt".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: Vec::new(),
            remote_addr: "127.0.0.1:1234".to_string(),
        };

        let response = route_request(
            request,
            &config,
            &HashMap::new(),
            &files,
            &[],
            None,
            "parent",
        )
        .await;
        assert_eq!(response.status_code, 200);
        assert_eq!(response.body, b"hello");
        assert_eq!(
            response.headers.get("content-type"),
            Some(&"text/plain; charset=utf-8".to_string())
        );

        let request = super::super::http_runtime::HttpRequest {
            method: "GET".to_string(),
            path: "/files/nested%2Fname.txt".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body: Vec::new(),
            remote_addr: "127.0.0.1:1234".to_string(),
        };
        let response = route_request(
            request,
            &config,
            &HashMap::new(),
            &files,
            &[],
            None,
            "parent",
        )
        .await;
        assert_eq!(response.status_code, 200);
        assert_eq!(response.body, b"encoded slash");
    }

    #[tokio::test]
    async fn rest_openapi_route_describes_registered_routes_and_auth() {
        let mut handler_node = Node::new(
            "message_handler",
            "Message Handler",
            "Creates a message",
            "Tests",
        );
        handler_node.add_output_pin("message", "Message", "Message text", VariableType::String);
        let handler = internal_node(handler_node);
        let parent = internal_node(RestServerNode::new().get_node());
        let context = test_context(parent, vec![handler.clone()]).await;
        let config = RestServerConfig {
            auth: RestAuthConfig::ApiKey {
                header: "x-api-key".to_string(),
                key: "secret".to_string(),
            },
            function_routes: vec![RestFunctionRoute {
                path: "/messages".to_string(),
                methods: vec!["POST".to_string()],
                function_refs: vec![handler.node_id().to_string()],
            }],
            openapi_routes: vec![RestOpenApiRoute {
                path: "/openapi.json".to_string(),
                ui_path: Some("/docs".to_string()),
            }],
            ..Default::default()
        };
        let specs = build_openapi_specs(&context, &config, "127.0.0.1:8080").await;
        assert_eq!(specs.len(), 2);
        let request = super::super::http_runtime::HttpRequest {
            method: "GET".to_string(),
            path: "/openapi.json".to_string(),
            query: HashMap::new(),
            headers: HashMap::from([("x-api-key".to_string(), "secret".to_string())]),
            body: Vec::new(),
            remote_addr: "127.0.0.1:1234".to_string(),
        };

        let response = route_request(
            request,
            &config,
            &HashMap::new(),
            &[],
            &specs,
            None,
            "parent",
        )
        .await;

        assert_eq!(response.status_code, 200);
        assert_eq!(
            response.headers.get("content-type").map(String::as_str),
            Some("application/vnd.oai.openapi+json; charset=utf-8")
        );
        let spec: Value = flow_like_types::json::from_slice(&response.body).unwrap();
        assert_eq!(spec["openapi"], json!("3.1.0"));
        assert!(
            spec["paths"]
                .as_object()
                .unwrap()
                .get("/openapi.json")
                .is_none()
        );
        assert_eq!(
            spec["components"]["securitySchemes"]["ApiKeyAuth"]["name"],
            json!("x-api-key")
        );
        assert_eq!(
            spec["paths"]["/messages"]["post"]["requestBody"]["content"]["application/json"]["schema"]
                ["properties"]["message"]["type"],
            json!("string")
        );
        assert_eq!(
            spec["paths"]["/messages"]["post"]["responses"]["401"]["description"],
            json!("Authentication required or token invalid")
        );
        assert!(!String::from_utf8(response.body).unwrap().contains("secret"));

        let ui_request = super::super::http_runtime::HttpRequest {
            method: "GET".to_string(),
            path: "/docs".to_string(),
            query: HashMap::new(),
            headers: HashMap::from([("x-api-key".to_string(), "secret".to_string())]),
            body: Vec::new(),
            remote_addr: "127.0.0.1:1234".to_string(),
        };
        let ui_response = route_request(
            ui_request,
            &config,
            &HashMap::new(),
            &[],
            &specs,
            None,
            "parent",
        )
        .await;
        assert_eq!(ui_response.status_code, 200);
        assert_eq!(
            ui_response.headers.get("content-type").map(String::as_str),
            Some("text/html; charset=utf-8")
        );
        let ui_body = String::from_utf8(ui_response.body).unwrap();
        assert!(ui_body.contains("SwaggerUIBundle"));
        assert!(ui_body.contains("SwaggerUIStandalonePreset"));
        assert!(ui_body.contains("tryItOutEnabled: true"));
        assert!(ui_body.contains("/openapi.json"));
    }

    #[tokio::test]
    async fn rest_openapi_request_schema_keeps_named_event_pins_when_payload_exists() {
        let mut handler_node = Node::new(
            "events_generic",
            "Generic Event",
            "A generic event without input or output",
            "Events",
        );
        handler_node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Starting an event",
            VariableType::Execution,
        );
        handler_node.add_output_pin("Name", "Name", "Person name", VariableType::String);
        handler_node.add_output_pin("Age", "Age", "Person age", VariableType::Integer);
        handler_node.add_output_pin("payload", "Payload", "The payload", VariableType::Struct);

        let handler = internal_node(handler_node);
        let parent = internal_node(RestServerNode::new().get_node());
        let context = test_context(parent, vec![handler.clone()]).await;
        let config = RestServerConfig {
            function_routes: vec![RestFunctionRoute {
                path: "/form".to_string(),
                methods: vec!["POST".to_string()],
                function_refs: vec![handler.node_id().to_string()],
            }],
            ..Default::default()
        };

        let spec = build_openapi_document(&context, &config, "127.0.0.1:8080").await;
        let schema =
            &spec["paths"]["/form"]["post"]["requestBody"]["content"]["application/json"]["schema"];

        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["properties"]["name"]["type"], json!("string"));
        assert_eq!(schema["properties"]["age"]["type"], json!("integer"));
        assert!(schema["properties"].get("payload").is_none());
        assert_eq!(
            schema["x-flow-like-payload-schema"]["additionalProperties"],
            json!(true)
        );
    }
}
