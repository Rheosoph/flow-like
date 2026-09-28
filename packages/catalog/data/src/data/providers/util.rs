use flow_like::flow::{
    execution::{ExecutionEnvironment, egress},
    node::Node,
};
use flow_like_storage::object_store::{
    self, ClientConfigKey, ClientOptions,
    aws::AmazonS3Builder,
    azure::MicrosoftAzureBuilder,
    client::{
        HttpClient, HttpConnector, HttpError, HttpErrorKind, HttpRequest, HttpResponse, HttpService,
    },
    gcp::GoogleCloudStorageBuilder,
};
use flow_like_types::{Value, async_trait, reqwest, reqwest::Url};
use std::time::Duration;

/// `ClientOptions`' own defaults; catalog stores never override them.
const STORE_TIMEOUT: Duration = Duration::from_secs(30);
const STORE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn get_pin_string_value(node: &Node, name: &str) -> String {
    node.get_pin_by_name(name)
        .and_then(|pin| pin.default_value.clone())
        .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
        .and_then(|v| v.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// Fails fast on a flow-supplied endpoint whose host, or server-side what it
/// currently resolves to, is on the host plane. Object stores additionally
/// get [`GuardedConnector`], which covers re-resolution and redirects; this
/// check alone is all third-party clients such as BigQuery's get.
pub async fn ensure_store_endpoint_allowed(
    environment: ExecutionEnvironment,
    provider: &str,
    endpoint: &str,
) -> flow_like_types::Result<()> {
    let url = Url::parse(endpoint.trim()).map_err(|error| {
        flow_like_types::anyhow!("{provider}: endpoint '{endpoint}' is not a valid URL: {error}")
    })?;
    egress::ensure_url_resolves_allowed(environment, &url).await
}

/// Refuses a value interpolated into a first-party host name
/// (`s3.{region}.amazonaws.com`, `{account}.blob.core.windows.net`, …) unless
/// it is made of host-name characters, so it cannot rewrite the host with
/// `/`, `@`, `:`, `?` or `#`.
pub fn ensure_host_fragment(
    provider: &str,
    field: &str,
    value: &str,
) -> flow_like_types::Result<()> {
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return Err(flow_like_types::anyhow!(
            "{provider}: {field} '{value}' must contain only letters, digits, '-' and '.'"
        ));
    }
    Ok(())
}

/// Server-side HTTP stack for object stores: every request URL is vetted,
/// and the guarded resolver and redirect policy refuse host-plane targets
/// reached through DNS rebinding or a redirect from an allowed endpoint.
#[derive(Debug, Clone, Copy)]
pub struct GuardedConnector(ExecutionEnvironment);

impl HttpConnector for GuardedConnector {
    fn connect(&self, options: &ClientOptions) -> object_store::Result<HttpClient> {
        let enabled = |key: ClientConfigKey| {
            options.get_config_value(&key).is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "true" | "1" | "yes" | "on"
                )
            })
        };
        let mut builder = egress::client_builder(self.0)
            .timeout(STORE_TIMEOUT)
            .connect_timeout(STORE_CONNECT_TIMEOUT)
            .http1_only()
            .no_gzip()
            .no_brotli()
            .no_zstd()
            .no_deflate()
            .https_only(!enabled(ClientConfigKey::AllowHttp));
        if let Some(agent) = options.get_config_value(&ClientConfigKey::UserAgent) {
            builder = builder.user_agent(agent);
        }
        if let Some(headers) = options.get_default_headers() {
            builder = builder.default_headers(headers.clone());
        }
        let client = builder
            .build()
            .map_err(|error| object_store::Error::Generic {
                store: "GuardedConnector",
                source: Box::new(error),
            })?;
        Ok(HttpClient::new(GuardedStoreService {
            client,
            environment: self.0,
        }))
    }
}

#[derive(Debug)]
struct GuardedStoreService {
    client: reqwest::Client,
    environment: ExecutionEnvironment,
}

#[async_trait]
impl HttpService for GuardedStoreService {
    async fn call(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let refused = |error: flow_like_types::Error| {
            HttpError::new(
                HttpErrorKind::Unknown,
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, error),
            )
        };
        let url = Url::parse(&request.uri().to_string()).map_err(|error| {
            refused(flow_like_types::anyhow!(
                "Invalid object store URL: {error}"
            ))
        })?;
        egress::ensure_url_allowed(self.environment, &url).map_err(refused)?;
        self.client.call(request).await
    }
}

/// Installs [`GuardedConnector`] on an object-store builder server-side;
/// locally the store keeps its own HTTP client.
pub trait WithEgressGuard: Sized {
    fn with_egress_guard(self, environment: ExecutionEnvironment) -> Self;
}

macro_rules! impl_with_egress_guard {
    ($($builder:ty),*) => {$(
        impl WithEgressGuard for $builder {
            fn with_egress_guard(self, environment: ExecutionEnvironment) -> Self {
                if environment == ExecutionEnvironment::Server {
                    self.with_http_connector(GuardedConnector(environment))
                } else {
                    self
                }
            }
        }
    )*};
}

impl_with_egress_guard!(
    AmazonS3Builder,
    MicrosoftAzureBuilder,
    GoogleCloudStorageBuilder
);

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::{ObjectStoreExt, path::Path};

    #[tokio::test]
    async fn guarded_stores_never_connect_to_the_host_plane_server_side() {
        let listener = flow_like_types::tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let store = AmazonS3Builder::new()
            .with_bucket_name("bucket")
            .with_region("us-east-1")
            .with_access_key_id("key")
            .with_secret_access_key("secret")
            .with_endpoint(format!("http://{}", listener.local_addr().unwrap()))
            .with_allow_http(true)
            .with_egress_guard(ExecutionEnvironment::Server)
            .build()
            .unwrap();

        let error = store.get(&Path::from("object")).await.unwrap_err();
        assert!(
            error.to_string().contains("refused"),
            "the loopback endpoint must be refused: {error}"
        );
        assert!(
            flow_like_types::tokio::time::timeout(Duration::from_millis(200), listener.accept())
                .await
                .is_err(),
            "no connection may reach the loopback listener"
        );
    }

    #[tokio::test]
    async fn guarded_stores_stay_https_only_unless_http_is_allowed() {
        let store = AmazonS3Builder::new()
            .with_bucket_name("bucket")
            .with_region("us-east-1")
            .with_access_key_id("key")
            .with_secret_access_key("secret")
            .with_endpoint("http://203.0.113.10:9")
            .with_egress_guard(ExecutionEnvironment::Server)
            .build()
            .unwrap();

        let result = flow_like_types::tokio::time::timeout(
            Duration::from_secs(2),
            store.get(&Path::from("object")),
        )
        .await
        .expect("a plain-http request is refused before connecting");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn store_endpoints_on_the_host_plane_are_refused_server_side() {
        for endpoint in [
            "http://169.254.169.254",
            "http://127.0.0.1:9000",
            "http://[fd00:ec2::254]",
            "http://metadata.google.internal",
            "http://localhost:4566",
        ] {
            assert!(
                ensure_store_endpoint_allowed(
                    ExecutionEnvironment::Server,
                    "AwsProvider",
                    endpoint
                )
                .await
                .is_err(),
                "{endpoint} must be refused server-side"
            );
            assert!(
                ensure_store_endpoint_allowed(ExecutionEnvironment::Local, "AwsProvider", endpoint)
                    .await
                    .is_ok()
            );
        }
        assert!(
            ensure_store_endpoint_allowed(
                ExecutionEnvironment::Server,
                "AwsProvider",
                "https://203.0.113.10:9000"
            )
            .await
            .is_ok()
        );
        assert!(
            ensure_store_endpoint_allowed(ExecutionEnvironment::Server, "AwsProvider", "not a url")
                .await
                .is_err()
        );
    }

    #[test]
    fn host_fragments_cannot_rewrite_the_host() {
        for value in [
            "eu-central-1",
            "mystorageaccount",
            "a1b2c3",
            "us-gov-west-1",
        ] {
            assert!(ensure_host_fragment("AwsProvider", "region", value).is_ok());
        }
        for value in [
            "",
            "@169.254.169.254/",
            "169.254.169.254/latest/meta-data?",
            "evil.example#",
            "host:8080",
            "a b",
        ] {
            assert!(
                ensure_host_fragment("AwsProvider", "region", value).is_err(),
                "{value:?} must be refused"
            );
        }
    }
}
