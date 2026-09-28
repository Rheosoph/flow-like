use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io, time::Duration};

pub mod cache;
pub mod lmtp;
pub mod ses;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

pub fn invalid(message: impl Into<String>) -> Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into()).into()
}

#[derive(Debug, Serialize)]
pub struct IngestRequest {
    pub source: &'static str,
    pub delivery_id: String,
    pub envelope_from: String,
    pub recipients: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_mime_base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3: Option<S3Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct S3Reference {
    pub bucket: String,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
}

impl IngestRequest {
    pub fn postfix(raw: &[u8], sender: &str, recipient: &str) -> Self {
        // Postfix adds its Received header before queueing. Hashing that header
        // with the message distinguishes independent receptions while retries
        // of the same queue file retain their identity. All recipients share
        // one identity so aliases of the same event do not run it twice.
        let mut hash = blake3::Hasher::new();
        for value in [raw, sender.as_bytes()] {
            hash.update(&(value.len() as u64).to_be_bytes());
            hash.update(value);
        }
        Self {
            source: "postfix",
            delivery_id: format!("postfix:{}", hash.finalize().to_hex()),
            envelope_from: sender.into(),
            recipients: vec![recipient.into()],
            raw_mime_base64: Some(STANDARD.encode(raw)),
            s3: None,
            authentication: None,
        }
    }
}

#[async_trait]
pub trait IngestApi: Send + Sync {
    async fn accepts(&self, recipient: &str) -> Result<bool>;
    async fn ingest(&self, request: &IngestRequest) -> Result<()>;
}

#[derive(Clone)]
pub struct ApiClient {
    client: reqwest::Client,
    base: reqwest::Url,
    token: String,
}

impl ApiClient {
    pub fn new(base: &str, token: String) -> Result<Self> {
        let base = reqwest::Url::parse(&format!("{}/", base.trim_end_matches('/')))?;
        if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() {
            return Err(invalid("API_BASE_URL must be an HTTP(S) URL"));
        }
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(invalid(
                "API_BASE_URL must not contain credentials, query, or fragment",
            ));
        }
        if token.trim().is_empty() {
            return Err(invalid("SINK_TRIGGER_JWT is empty"));
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            base,
            token,
        })
    }

    pub async fn dispatch(&self) -> Result<()> {
        let body = b"{}";
        let response = self
            .client
            .post(self.base.join("api/v1/sink/mail/dispatch")?)
            .timeout(Duration::from_secs(75))
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .header(
                "x-amz-content-sha256",
                format!("{:x}", Sha256::digest(body)),
            )
            .body(body.as_slice())
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(invalid(format!(
                "Mail dispatch returned {}",
                response.status()
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl IngestApi for ApiClient {
    async fn accepts(&self, recipient: &str) -> Result<bool> {
        let mut url = self.base.join("api/v1/sink/mail/recipients/")?;
        url.path_segments_mut()
            .map_err(|_| invalid("Invalid API URL"))?
            .pop_if_empty()
            .push(recipient);
        let response = self.client.get(url).bearer_auth(&self.token).send().await?;
        if !response.status().is_success() {
            return Err(invalid(format!(
                "Recipient lookup returned {}",
                response.status()
            )));
        }
        #[derive(Deserialize)]
        struct Acceptance {
            accepted: bool,
        }
        Ok(response.json::<Acceptance>().await?.accepted)
    }

    async fn ingest(&self, request: &IngestRequest) -> Result<()> {
        // CloudFront OAC requires the payload hash for Lambda URL POSTs.
        // Hash the same serialized bytes that reqwest sends on the wire.
        let body = serde_json::to_vec(request)?;
        let response = self
            .client
            .post(self.base.join("api/v1/sink/mail/ingest")?)
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .header(
                "x-amz-content-sha256",
                format!("{:x}", Sha256::digest(&body)),
            )
            .body(body)
            .send()
            .await?;
        // This status is the API's durable-acceptance contract. Never treat a
        // redirect, authentication failure, or generic 200 page as acceptance.
        if response.status() != reqwest::StatusCode::ACCEPTED {
            return Err(invalid(format!(
                "Mail ingestion returned {}",
                response.status()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_identity_tracks_reception_and_is_shared_by_aliases() {
        let raw = b"Received: by mx.example.test with ESMTP id ABC; today\r\n\r\nhello";
        let first = IngestRequest::postfix(raw, "sender@example.test", "one@example.test");
        assert_eq!(
            first.delivery_id,
            IngestRequest::postfix(raw, "sender@example.test", "one@example.test").delivery_id
        );
        assert_eq!(
            first.delivery_id,
            IngestRequest::postfix(raw, "sender@example.test", "two@example.test").delivery_id
        );
        assert_eq!(first.recipients, ["one@example.test"]);
        assert_ne!(
            first.delivery_id,
            IngestRequest::postfix(raw, "other@example.test", "one@example.test").delivery_id
        );
        assert_ne!(
            first.delivery_id,
            IngestRequest::postfix(
                b"Received: by mx.example.test with ESMTP id DEF; today\r\n\r\nhello",
                "sender@example.test",
                "one@example.test"
            )
            .delivery_id
        );
    }

    #[tokio::test]
    async fn posts_hash_the_exact_wire_body_for_cloudfront_oac() {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (path, status) in [("ingest", "202 Accepted"), ("dispatch", "200 OK")] {
                let (stream, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                assert_eq!(line, format!("POST /api/v1/sink/mail/{path} HTTP/1.1\r\n"));
                let mut headers = std::collections::HashMap::new();
                loop {
                    line.clear();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    let (key, value) = line.trim_end().split_once(':').unwrap();
                    headers.insert(key.to_ascii_lowercase(), value.trim().to_string());
                }
                let length: usize = headers["content-length"].parse().unwrap();
                let mut body = vec![0; length];
                reader.read_exact(&mut body).await.unwrap();
                assert_eq!(headers["authorization"], "Bearer scoped-token");
                assert_eq!(headers["content-type"], "application/json");
                assert_eq!(
                    headers["x-amz-content-sha256"],
                    format!("{:x}", Sha256::digest(&body))
                );
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                if path == "ingest" {
                    assert_eq!(body["recipients"][0], "bcc@example.test");
                } else {
                    assert_eq!(body, serde_json::json!({}));
                }
                reader.get_mut().write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").as_bytes()).await.unwrap();
            }
        });
        let client = ApiClient::new(&base, "scoped-token".into()).unwrap();
        client
            .ingest(&IngestRequest::postfix(
                b"Subject: test\r\n\r\nbody",
                "sender@example.test",
                "bcc@example.test",
            ))
            .await
            .unwrap();
        client.dispatch().await.unwrap();
        server.await.unwrap();
    }
}
