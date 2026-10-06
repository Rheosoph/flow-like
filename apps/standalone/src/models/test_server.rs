//! An HTTPS origin on loopback for the fetch and acquisition tests.
use super::fetch::{AddressPolicy, Fetcher};
use anyhow::Result;
use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{MethodRouter, get},
};
use flow_like_device_protocol::{DigestAlgorithm, ModelAssetDescriptor, ModelAssetDigest};
use futures_util::StreamExt;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, server::TlsStream};

#[derive(Clone, Debug)]
pub struct Hit {
    pub path: String,
    pub range: Option<String>,
    pub accept_encoding: Option<String>,
}

#[derive(Clone, Default)]
pub struct Hits(Arc<Mutex<Vec<Hit>>>);

impl Hits {
    pub fn to(&self, path: &str) -> Vec<Hit> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|hit| hit.path == path)
            .cloned()
            .collect()
    }

    pub fn total(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

pub struct Origin {
    pub port: u16,
    pub certificate: String,
    pub hits: Hits,
}

struct TlsListener {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let Ok((stream, address)) = self.listener.accept().await else {
                continue;
            };
            if let Ok(stream) = self.acceptor.accept(stream).await {
                return (stream, address);
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

fn header_value(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

async fn record(State(hits): State<Hits>, request: Request, next: Next) -> Response {
    let hit = Hit {
        path: request.uri().path().to_owned(),
        range: header_value(request.headers(), header::RANGE),
        accept_encoding: header_value(request.headers(), header::ACCEPT_ENCODING),
    };
    hits.0.lock().unwrap().push(hit);
    next.run(request).await
}

impl Origin {
    pub async fn start(router: Router) -> Self {
        Self::listen(router).await.expect("an HTTPS test origin")
    }

    async fn listen(router: Router) -> Result<Self> {
        let identity =
            rcgen::generate_simple_self_signed(vec!["127.0.0.1".into(), "localhost".into()])?;
        let config =
            rustls::ServerConfig::builder_with_provider(Arc::new(crate::crypto::tls_provider()))
                .with_safe_default_protocol_versions()?
                .with_no_client_auth()
                .with_single_cert(
                    vec![identity.cert.der().clone()],
                    rustls::pki_types::PrivatePkcs8KeyDer::from(
                        identity.signing_key.serialize_der(),
                    )
                    .into(),
                )?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let hits = Hits::default();
        let router = router.layer(middleware::from_fn_with_state(hits.clone(), record));
        let listener = TlsListener {
            listener,
            acceptor: TlsAcceptor::from(Arc::new(config)),
        };
        tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self {
            port,
            certificate: identity.cert.pem(),
            hits,
        })
    }

    pub fn url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{path}", self.port)
    }

    /// Trusts this origin and permits its loopback address only.
    pub fn fetcher(&self) -> Fetcher {
        Fetcher::trusting(
            AddressPolicy::allowing([IpAddr::V4(Ipv4Addr::LOCALHOST)]),
            &self.certificate,
        )
        .expect("a fetcher that trusts the test origin")
    }
}

pub fn pattern(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index * 31 % 251) as u8).collect()
}

pub fn asset(bytes: &[u8], sources: Vec<String>) -> ModelAssetDescriptor {
    ModelAssetDescriptor {
        digest: ModelAssetDigest {
            algorithm: DigestAlgorithm::Blake3,
            hex: blake3::hash(bytes).to_hex().to_string(),
        },
        size: bytes.len() as u64,
        file_name: "model.gguf".into(),
        sources,
    }
}

fn range_start(headers: &HeaderMap) -> Option<usize> {
    headers
        .get(header::RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes=")?
        .strip_suffix('-')?
        .parse()
        .ok()
}

/// Answers whole or open-ended ranged requests for `bytes`.
pub fn ranged(bytes: &[u8], headers: &HeaderMap) -> Response {
    match range_start(headers) {
        None => bytes.to_vec().into_response(),
        Some(start) if start < bytes.len() => (
            StatusCode::PARTIAL_CONTENT,
            [(
                header::CONTENT_RANGE,
                format!("bytes {start}-{}/{}", bytes.len() - 1, bytes.len()),
            )],
            bytes[start..].to_vec(),
        )
            .into_response(),
        Some(_) => StatusCode::RANGE_NOT_SATISFIABLE.into_response(),
    }
}

pub fn serving(bytes: Arc<Vec<u8>>) -> MethodRouter {
    get(move |headers: HeaderMap| {
        let bytes = Arc::clone(&bytes);
        async move { ranged(&bytes, &headers) }
    })
}

/// Announces all of `bytes` but sends only the first `at`, then breaks the connection,
/// or keeps it open without sending more when `stall`. The break waits until the head
/// has been flushed, since aborting drops what is still buffered.
pub fn truncated(bytes: &[u8], at: usize, stall: bool) -> Response {
    let head = futures_util::stream::iter([Ok::<_, std::io::Error>(
        bytes::Bytes::copy_from_slice(&bytes[..at]),
    )]);
    let cut = futures_util::stream::once(async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        Err(std::io::Error::other("connection cut"))
    });
    let body = if stall {
        Body::from_stream(head.chain(futures_util::stream::pending()))
    } else {
        Body::from_stream(head.chain(cut))
    };
    Response::builder()
        .header(header::CONTENT_LENGTH, bytes.len())
        .body(body)
        .expect("a truncated test response builds")
}

/// Announces all of `bytes` and sends them one byte per `every`, like a source that trickles.
pub fn trickling(bytes: &[u8], every: std::time::Duration) -> Response {
    let length = bytes.len();
    let bytes = bytes::Bytes::copy_from_slice(bytes);
    let body = futures_util::stream::unfold(0, move |sent| {
        let next = (sent < bytes.len()).then(|| bytes.slice(sent..=sent));
        async move {
            tokio::time::sleep(every).await;
            next.map(|byte| (Ok::<_, std::io::Error>(byte), sent + 1))
        }
    });
    Response::builder()
        .header(header::CONTENT_LENGTH, length)
        .body(Body::from_stream(body))
        .expect("a trickling test response builds")
}
