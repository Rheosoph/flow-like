//! Cluster and admission transports. Ambiguous mutations are never replayed.
use async_trait::async_trait;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, Semaphore};

#[derive(Debug, Clone)]
pub enum TransportError {
    Status(u16),
    Unavailable,
    Invalid,
}

pub type TransportResult<T> = std::result::Result<T, TransportError>;

#[async_trait]
pub trait KubeApi: Send + Sync {
    async fn request(
        &self,
        method: Method,
        kind: &str,
        name: Option<&str>,
        body: Option<Value>,
        query: &[(&str, String)],
    ) -> TransportResult<Value>;

    async fn get(&self, kind: &str, name: &str) -> TransportResult<Option<Value>> {
        match self.request(Method::GET, kind, Some(name), None, &[]).await {
            Ok(value) => Ok(Some(value)),
            Err(TransportError::Status(404)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn list(&self, kind: &str, selector: &str) -> TransportResult<Vec<Value>> {
        let mut items = Vec::new();
        let mut cursor = String::new();
        loop {
            let mut query = vec![
                ("labelSelector", selector.to_owned()),
                ("limit", "500".to_owned()),
            ];
            if !cursor.is_empty() {
                query.push(("continue", cursor));
            }
            let page = self.request(Method::GET, kind, None, None, &query).await?;
            items.extend(
                page["items"]
                    .as_array()
                    .ok_or(TransportError::Invalid)?
                    .iter()
                    .cloned(),
            );
            cursor = page["metadata"]["continue"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if cursor.is_empty() {
                return Ok(items);
            }
        }
    }

    async fn delete(&self, kind: &str, object: &Value) -> TransportResult<()> {
        let name = object["metadata"]["name"]
            .as_str()
            .ok_or(TransportError::Invalid)?;
        let uid = object["metadata"]["uid"]
            .as_str()
            .ok_or(TransportError::Invalid)?;
        let result = self.request(Method::DELETE, kind, Some(name), Some(json!({
            "apiVersion":"v1", "kind":"DeleteOptions", "preconditions":{"uid":uid}, "propagationPolicy":"Foreground"
        })), &[]).await;
        match result {
            Ok(_) | Err(TransportError::Status(404)) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

pub struct Kube {
    client: Client,
    base: reqwest::Url,
    namespace: String,
    token_file: PathBuf,
    token: Mutex<Option<(std::time::Instant, String)>>,
    requests: Semaphore,
}

impl Kube {
    pub async fn new(namespace: &str, host: &str, port: u16) -> TransportResult<Self> {
        let root = PathBuf::from("/var/run/secrets/kubernetes.io/serviceaccount");
        let ca = tokio::fs::read(root.join("ca.crt"))
            .await
            .map_err(|_| TransportError::Unavailable)?;
        let certificate =
            reqwest::Certificate::from_pem(&ca).map_err(|_| TransportError::Invalid)?;
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        let base = reqwest::Url::parse(&format!("https://{host}:{port}"))
            .map_err(|_| TransportError::Invalid)?;
        let client = Client::builder()
            .no_proxy()
            .https_only(true)
            .tls_built_in_root_certs(false)
            .add_root_certificate(certificate)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .pool_max_idle_per_host(32)
            .pool_idle_timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| TransportError::Invalid)?;
        Ok(Self {
            client,
            base,
            namespace: namespace.into(),
            token_file: root.join("token"),
            token: Mutex::new(None),
            requests: Semaphore::new(32),
        })
    }
}

#[async_trait]
impl KubeApi for Kube {
    async fn request(
        &self,
        method: Method,
        kind: &str,
        name: Option<&str>,
        body: Option<Value>,
        query: &[(&str, String)],
    ) -> TransportResult<Value> {
        let _permit = self
            .requests
            .acquire()
            .await
            .map_err(|_| TransportError::Unavailable)?;
        let mut url = self.base.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| TransportError::Invalid)?;
            if kind == "networkpolicies" {
                path.extend(["apis", "networking.k8s.io", "v1"]);
            } else if matches!(kind, "pods" | "configmaps") {
                path.extend(["api", "v1"]);
            } else {
                return Err(TransportError::Invalid);
            }
            path.extend(["namespaces", &self.namespace, kind]);
            if let Some(name) = name {
                path.push(name);
            }
        }
        url.query_pairs_mut()
            .extend_pairs(query.iter().map(|(key, value)| (*key, value)));
        // Projected tokens rotate. Bound the cache lifetime while keeping
        // filesystem work outside the usual assignment path.
        let token = {
            let mut cached = self.token.lock().await;
            if cached
                .as_ref()
                .is_none_or(|(loaded, _)| loaded.elapsed() > Duration::from_secs(30))
            {
                let token = tokio::fs::read_to_string(&self.token_file)
                    .await
                    .map_err(|_| TransportError::Unavailable)?;
                *cached = Some((std::time::Instant::now(), token));
            }
            cached
                .as_ref()
                .expect("loaded service account token")
                .1
                .clone()
        };
        let content_type = if method == Method::PATCH {
            "application/merge-patch+json"
        } else {
            "application/json"
        };
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(token.trim())
            .header("Content-Type", content_type);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| TransportError::Unavailable)?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| TransportError::Unavailable)?
        {
            if bytes.len() + chunk.len() > 32 * 1024 * 1024 {
                return Err(TransportError::Invalid);
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(TransportError::Status(status.as_u16()));
        }
        if bytes.is_empty() || status == StatusCode::NO_CONTENT {
            Ok(json!({}))
        } else {
            serde_json::from_slice(&bytes).map_err(|_| TransportError::Invalid)
        }
    }
}

#[async_trait]
pub trait ClaimStore: Send + Sync {
    async fn ping(&self) -> TransportResult<()>;
    async fn claim(&self, run_id: &str, slot_id: &str) -> TransportResult<bool>;
}

pub struct Claims {
    client: redis::Client,
    connection: Mutex<Option<redis::aio::MultiplexedConnection>>,
    concurrent: Semaphore,
    prefix: String,
    ttl: u64,
}

impl Claims {
    pub fn new(
        url: &str,
        namespace: &str,
        installation: &str,
        ttl: u64,
    ) -> TransportResult<Arc<Self>> {
        validate_redis_url(url)?;
        let client = redis::Client::open(url).map_err(|_| TransportError::Invalid)?;
        Ok(Arc::new(Self {
            client,
            connection: Mutex::new(None),
            concurrent: Semaphore::new(32),
            prefix: format!("exec:claims:v1:{namespace}:{installation}:"),
            ttl,
        }))
    }

    async fn connection(&self) -> TransportResult<redis::aio::MultiplexedConnection> {
        let mut connection = self.connection.lock().await;
        if let Some(connection) = connection.as_ref() {
            return Ok(connection.clone());
        }
        let config = redis::AsyncConnectionConfig::new()
            .set_connection_timeout(Duration::from_secs(2))
            .set_response_timeout(Duration::from_secs(2));
        let next = self
            .client
            .get_multiplexed_async_connection_with_config(&config)
            .await
            .map_err(|_| TransportError::Unavailable)?;
        *connection = Some(next.clone());
        Ok(next)
    }

    async fn command<T: redis::FromRedisValue>(&self, command: &redis::Cmd) -> TransportResult<T> {
        let _permit = self
            .concurrent
            .acquire()
            .await
            .map_err(|_| TransportError::Unavailable)?;
        let mut connection = self.connection().await?;
        match command.query_async(&mut connection).await {
            Ok(value) => Ok(value),
            Err(_) => {
                // Reconnect for a later operation, but do not repeat this command.
                *self.connection.lock().await = None;
                Err(TransportError::Unavailable)
            }
        }
    }

    fn claim_command(&self, run_id: &str, slot_id: &str) -> redis::Cmd {
        let mut command = redis::cmd("SET");
        command
            .arg(format!(
                "{}{:x}",
                self.prefix,
                Sha256::digest(run_id.as_bytes())
            ))
            .arg(slot_id)
            .arg("NX")
            .arg("EX")
            .arg(self.ttl);
        command
    }
}

fn validate_redis_url(value: &str) -> TransportResult<()> {
    let url = reqwest::Url::parse(value).map_err(|_| TransportError::Invalid)?;
    if !matches!(url.scheme(), "redis" | "rediss")
        || url.host_str().is_none()
        || url.password().is_none_or(str::is_empty)
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(TransportError::Invalid);
    }
    Ok(())
}

#[async_trait]
impl ClaimStore for Claims {
    async fn ping(&self) -> TransportResult<()> {
        let value: String = self.command(&redis::cmd("PING")).await?;
        if value == "PONG" {
            Ok(())
        } else {
            Err(TransportError::Unavailable)
        }
    }
    async fn claim(&self, run_id: &str, slot_id: &str) -> TransportResult<bool> {
        let value: Option<String> = self.command(&self.claim_command(run_id, slot_id)).await?;
        Ok(value.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    #[test]
    fn redis_configuration_requires_authentication_and_cannot_disable_tls_checks() {
        assert!(validate_redis_url("rediss://user:password@redis.example:6380/0").is_ok());
        for url in [
            "redis://redis/0",
            "rediss://u:p@redis/0?ssl_cert_reqs=none",
            "rediss://u:p@redis/0#insecure",
            "http://u:p@redis/",
        ] {
            assert!(validate_redis_url(url).is_err());
        }
    }

    #[test]
    fn admission_is_one_atomic_namespaced_set_with_retained_expiry() {
        let claims = Claims::new(
            "redis://u:p@localhost/0",
            "namespace",
            "installation",
            91000,
        )
        .unwrap();
        let packed =
            String::from_utf8(claims.claim_command("run", "slot").get_packed_command()).unwrap();
        assert!(packed.contains("exec:claims:v1:namespace:installation:"));
        assert!(packed.contains("\r\nNX\r\n"));
        assert!(packed.contains("\r\nEX\r\n"));
        assert!(packed.contains("\r\n91000\r\n"));
        assert_eq!(packed.matches("\r\nSET\r\n").count(), 1);
    }

    async fn redis_command(reader: &mut BufReader<tokio::net::TcpStream>) -> Option<Vec<String>> {
        let mut header = String::new();
        if reader.read_line(&mut header).await.ok()? == 0 {
            return None;
        }
        let size = header.strip_prefix('*')?.trim().parse::<usize>().ok()?;
        let mut command = Vec::new();
        for _ in 0..size {
            header.clear();
            reader.read_line(&mut header).await.ok()?;
            let size = header.strip_prefix('$')?.trim().parse::<usize>().ok()?;
            let mut value = vec![0; size + 2];
            reader.read_exact(&mut value).await.ok()?;
            command.push(String::from_utf8(value[..size].to_vec()).ok()?);
        }
        Some(command)
    }

    #[tokio::test]
    async fn lost_redis_reply_is_not_retried_and_next_call_preserves_the_claim() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = calls.clone();
        let server = tokio::spawn(async move {
            let mut recorded = None;
            for connection in 0..2 {
                let (socket, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(socket);
                while let Some(command) = redis_command(&mut reader).await {
                    if command[0] == "SET" {
                        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        if connection == 0 {
                            recorded = Some(command[1].clone());
                            break;
                        }
                        assert_eq!(recorded.as_ref(), Some(&command[1]));
                        reader.get_mut().write_all(b"$-1\r\n").await.unwrap();
                        return;
                    }
                    reader.get_mut().write_all(b"+OK\r\n").await.unwrap();
                }
            }
        });
        let claims = Claims::new(
            &format!("redis://user:password@{address}/0"),
            "namespace",
            "installation",
            91000,
        )
        .unwrap();
        assert!(claims.claim("run", "first-slot").await.is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!claims.claim("run", "second-slot").await.unwrap());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
}
