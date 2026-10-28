//! Downloads of media links that callers hand the API for a model: links into the platform's
//! own object store and public HTTPS.
//!
//! The object store may sit on the API's internal network, so a link into it is fetched as it
//! is, without redirects, and the store's signature check decides what it serves. Any other link
//! must pass the server egress guard, on every redirect as well. Fetches are bounded in time and
//! size, and links are never logged: a signed link is a credential.

use std::{fmt, time::Duration};

use flow_like::flow_like_model_provider::egress::{EgressRefused, ProviderEgress};
use flow_like_storage::{Path, files::store::FlowLikeStore};
use flow_like_types::reqwest::{self, Client, ClientBuilder, Response, Url, redirect};

/// Object name a scope probe is signed for. The probe link is never fetched.
const PROBE_OBJECT: &str = "link-scope-probe";
const PROBE_TTL: Duration = Duration::from_secs(60);
const MAX_REDIRECTS: usize = 5;

/// Where the store points the links it signs for keys under a prefix: its origin and the path
/// in front of those keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StorageScope {
    origin: String,
    path_prefix: String,
}

impl StorageScope {
    /// Signs a probe under `prefix`, empty or a key prefix ending in `/`, as the store signs a
    /// download link. `None` when the store's links do not end in the object key.
    pub(crate) async fn signed_by(
        store: &FlowLikeStore,
        prefix: &str,
    ) -> flow_like_types::Result<Option<Self>> {
        let probe = Path::from(format!("{prefix}{PROBE_OBJECT}"));
        let signed = store.sign("GET", &probe, PROBE_TTL).await?;
        Ok(Self::from_probe(&signed))
    }

    fn from_probe(probe: &Url) -> Option<Self> {
        let path_prefix = probe
            .path()
            .strip_suffix(PROBE_OBJECT)
            .filter(|prefix| prefix.ends_with('/'))?;
        Some(Self {
            origin: tuple_origin(probe)?,
            path_prefix: path_prefix.to_owned(),
        })
    }

    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    pub(crate) fn contains(&self, url: &Url) -> bool {
        self.key(url).is_some()
    }

    /// The object key a link into this scope names, as the link encodes it.
    pub(crate) fn key<'a>(&self, url: &'a Url) -> Option<&'a str> {
        let same_origin = tuple_origin(url).is_some_and(|origin| origin == self.origin);
        if !same_origin || !url.username().is_empty() || url.password().is_some() {
            return None;
        }
        url.path()
            .strip_prefix(self.path_prefix.as_str())
            .filter(|key| is_object_key(key))
    }
}

fn tuple_origin(url: &Url) -> Option<String> {
    let origin = url.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

/// Non-empty segments of URL path characters and percent escapes. An escaped separator or NUL,
/// or a dot segment, could make the store read another key, so none counts.
fn is_object_key(key: &str) -> bool {
    key.split('/').all(|segment| {
        let decoded = urlencoding::decode_binary(segment.as_bytes());
        !segment.is_empty()
            && !matches!(decoded.as_ref(), b"." | b"..")
            && !decoded.iter().any(|byte| matches!(byte, b'/' | b'\\' | 0))
            && segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || "-._~!$&'()*+,;=:@%".contains(ch))
    })
}

/// How a link is fetched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// A link into the platform's own store, fetched as it is even when the store is internal.
    OwnStorage,
    /// Any other link, under the server egress guard.
    Guarded(ProviderEgress),
}

/// How `url` is fetched: as a link into `own` storage, or under `egress`. Names pass the
/// guard's check here; one that resolves to a refused address fails when the client connects.
pub(crate) fn route(
    url: &Url,
    own: Option<&StorageScope>,
    egress: ProviderEgress,
) -> Result<Route, EgressRefused> {
    if own.is_some_and(|own| own.contains(url)) {
        return Ok(Route::OwnStorage);
    }
    egress.check_url(url).map(|()| Route::Guarded(egress))
}

impl Route {
    /// The own store's client follows no redirect. The guarded client resolves every name
    /// through the guard, ignores proxies and follows a redirect only where the guard allows.
    pub(crate) fn client(self) -> reqwest::Result<Client> {
        match self {
            Self::OwnStorage => Client::builder().redirect(redirect::Policy::none()).build(),
            Self::Guarded(egress) => guarded_client(egress, Client::builder()),
        }
    }
}

fn guarded_client(egress: ProviderEgress, builder: ClientBuilder) -> reqwest::Result<Client> {
    egress
        .guard(builder)
        .redirect(redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            match egress.check_url(attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(refused) => attempt.error(refused),
            }
        }))
        .build()
}

/// Why a fetch failed, without the URL.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FetchError {
    TimedOut,
    TooLarge,
    Status(u16),
    Transport(String),
}

impl FetchError {
    /// The error and its causes, without the URL.
    pub(crate) fn transport(error: reqwest::Error) -> Self {
        let error = error.without_url();
        let chain = std::iter::successors(
            Some(&error as &(dyn std::error::Error + 'static)),
            |cause| cause.source(),
        );
        Self::Transport(
            chain
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(": "),
        )
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimedOut => formatter.write_str("timed out"),
            Self::TooLarge => formatter.write_str("exceeds the size limit"),
            Self::Status(status) => write!(formatter, "answered HTTP {status}"),
            Self::Transport(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for FetchError {}

/// One GET, bounded by `timeout` from connecting to the last body byte.
pub(crate) async fn fetch(
    client: &Client,
    url: Url,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Vec<u8>, FetchError> {
    let download = async {
        let response = client
            .get(url)
            .send()
            .await
            .map_err(FetchError::transport)?;
        let status = response.status();
        if !status.is_success() {
            return Err(FetchError::Status(status.as_u16()));
        }
        read_capped(response, max_bytes).await
    };
    flow_like_types::tokio::time::timeout(timeout, download)
        .await
        .unwrap_or(Err(FetchError::TimedOut))
}

/// Reads the body chunk by chunk and gives up as soon as it passes `max_bytes`.
async fn read_capped(mut response: Response, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(FetchError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(FetchError::transport)? {
        if body.len() + chunk.len() > max_bytes {
            return Err(FetchError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use bytes::Bytes;
    use flow_like_storage::object_store::{aws::AmazonS3Builder, azure::MicrosoftAzureBuilder};
    use flow_like_types::tokio::{
        self,
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    use std::{
        net::SocketAddr,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    fn url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    fn uploads_at(origin: &str) -> StorageScope {
        StorageScope::from_probe(&url(&format!(
            "{origin}/content/tmp/user/me/{PROBE_OBJECT}?X-Amz-Signature=probe"
        )))
        .unwrap()
    }

    #[test]
    fn the_scope_is_read_from_the_probe_link() {
        assert_eq!(
            uploads_at("http://rustfs:9000"),
            StorageScope {
                origin: "http://rustfs:9000".into(),
                path_prefix: "/content/tmp/user/me/".into(),
            }
        );
        let virtual_hosted = url(&format!(
            "https://content.s3.eu-central-1.amazonaws.com/{PROBE_OBJECT}"
        ));
        assert_eq!(
            StorageScope::from_probe(&virtual_hosted),
            Some(StorageScope {
                origin: "https://content.s3.eu-central-1.amazonaws.com".into(),
                path_prefix: "/".into(),
            })
        );
        for raw in [
            "https://content.example/tmp/user/me/other-object".to_string(),
            format!("https://content.example/tmp/user/me-{PROBE_OBJECT}"),
            format!("data:text/plain;base64,{PROBE_OBJECT}"),
        ] {
            assert_eq!(StorageScope::from_probe(&url(&raw)), None, "{raw}");
        }
    }

    #[test]
    fn links_into_the_store_route_to_it_even_when_it_is_internal() {
        let own = uploads_at("http://rustfs:9000");
        for raw in [
            "http://rustfs:9000/content/tmp/user/me/2026/10/28/k3x9.png?X-Amz-Signature=abc",
            "HTTP://RUSTFS:9000/content/tmp/user/me/apps/app-1/2026/10/28/k3x9.jpg",
            "http://rustfs:9000/content/tmp/user/me/files/%C3%9Cbersicht%20(2)%231.pdf",
        ] {
            assert_eq!(
                route(&url(raw), Some(&own), ProviderEgress::Public),
                Ok(Route::OwnStorage),
                "{raw}"
            );
        }
        assert_eq!(
            own.key(&url(
                "http://rustfs:9000/content/tmp/user/me/files/a%20b.pdf?X-Amz-Signature=abc"
            )),
            Some("files/a%20b.pdf")
        );
    }

    #[test]
    fn other_links_must_pass_the_egress_guard() {
        for own in [None, Some(uploads_at("http://rustfs:9000"))] {
            assert_eq!(
                route(
                    &url("https://images.example.com/cat.png"),
                    own.as_ref(),
                    ProviderEgress::Public
                ),
                Ok(Route::Guarded(ProviderEgress::Public))
            );
        }
        let own = uploads_at("http://rustfs:9000");
        for raw in [
            "http://rustfs:9000/content/tmp/user/other/k3x9.png",
            "http://rustfs:9000/content/tmp/user/me/../other/k3x9.png",
            "http://rustfs:9000/content/tmp/user/me/%2e%2e/other/k3x9.png",
            "http://rustfs:9000/content/tmp/user/me/..%2Fother%2Fk3x9.png",
            "http://rustfs:9000/content/tmp/user/me/k3x9%00.png",
            "http://rustfs:9000/content/tmp/user/me/",
            "http://rustfs:9000/content/tmp/user/me//k3x9.png",
            "http://rustfs:9000/other-bucket/tmp/user/me/k3x9.png",
            "http://rustfs:9000/minio/admin/v3/info",
            "http://rustfs:9001/content/tmp/user/me/k3x9.png",
            "http://key:secret@rustfs:9000/content/tmp/user/me/k3x9.png",
            "http://images.example.com/cat.png",
            "https://169.254.169.254/latest/meta-data/",
            "https://10.0.0.5/cat.png",
            "https://127.0.0.1:8443/cat.png",
            "https://2130706433/cat.png",
            "https://[::1]/cat.png",
            "https://localhost/cat.png",
            "https://metadata.google.internal/computeMetadata/v1/",
        ] {
            assert!(
                route(&url(raw), Some(&own), ProviderEgress::Public).is_err(),
                "{raw}"
            );
        }
        assert_eq!(
            route(
                &url("http://10.0.0.5:8000/cat.png"),
                None,
                ProviderEgress::Private
            ),
            Ok(Route::Guarded(ProviderEgress::Private))
        );
    }

    pub(crate) fn s3_store(endpoint: &str) -> FlowLikeStore {
        FlowLikeStore::AWS(Arc::new(
            AmazonS3Builder::new()
                .with_bucket_name("content")
                .with_region("us-east-1")
                .with_endpoint(endpoint)
                .with_allow_http(true)
                .with_access_key_id("test-access-key")
                .with_secret_access_key("test-secret-key")
                .build()
                .unwrap(),
        ))
    }

    #[flow_like_types::tokio::test]
    async fn links_the_store_signs_are_in_its_scope_and_others_are_not() {
        let stores = [
            s3_store("http://rustfs:9000"),
            FlowLikeStore::AWS(Arc::new(
                AmazonS3Builder::new()
                    .with_bucket_name("content")
                    .with_region("eu-central-1")
                    .with_virtual_hosted_style_request(true)
                    .with_access_key_id("test-access-key")
                    .with_secret_access_key("test-secret-key")
                    .build()
                    .unwrap(),
            )),
            FlowLikeStore::Azure(Arc::new(
                MicrosoftAzureBuilder::new()
                    .with_account("flowlike")
                    .with_container_name("content")
                    .with_access_key(STANDARD.encode("test-account-key"))
                    .build()
                    .unwrap(),
            )),
        ];
        for store in stores {
            let mine = StorageScope::signed_by(&store, "tmp/user/me/")
                .await
                .unwrap()
                .unwrap();
            let bucket = StorageScope::signed_by(&store, "").await.unwrap().unwrap();
            let sign = |key: &str| {
                let store = &store;
                let key = Path::from(key);
                async move {
                    store
                        .sign("GET", &key, Duration::from_secs(600))
                        .await
                        .unwrap()
                }
            };
            let upload = sign("tmp/user/me/2026/10/28/k3x9.png").await;
            let foreign = sign("tmp/user/someone-else/2026/10/28/k3x9.png").await;
            let app_file = sign("apps/app-1/storage/Übersicht (2)#1.pdf").await;
            assert!(mine.contains(&upload), "{mine:?}");
            assert!(!mine.contains(&foreign), "{mine:?}");
            assert!(!mine.contains(&app_file), "{mine:?}");
            for link in [&upload, &foreign, &app_file] {
                assert!(bucket.contains(link), "{bucket:?}");
            }
        }
    }

    fn streamed<S>(chunks: S) -> Response
    where
        S: futures::Stream<Item = std::io::Result<Bytes>> + Send + 'static,
    {
        Response::from(axum::http::Response::new(reqwest::Body::wrap_stream(
            chunks,
        )))
    }

    #[flow_like_types::tokio::test]
    async fn the_size_cap_holds_while_streaming() {
        let endless = futures::stream::repeat_with(|| Ok(Bytes::from_static(&[7; 1024])));
        assert_eq!(
            read_capped(streamed(endless), 64 * 1024).await,
            Err(FetchError::TooLarge)
        );
        let exact = futures::stream::iter((0..64).map(|_| Ok(Bytes::from_static(&[7; 1024]))));
        assert_eq!(
            read_capped(streamed(exact), 64 * 1024)
                .await
                .map(|body| body.len()),
            Ok(64 * 1024)
        );
    }

    /// Answers every connection on loopback with `response`, keeps the socket open until the
    /// client hangs up, and counts the connections.
    pub(crate) async fn serve(response: String) -> (SocketAddr, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = Arc::new(AtomicUsize::new(0));
        let count = served.clone();
        let response = Arc::new(response);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                let response = response.clone();
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    let _ = socket.read(&mut buffer).await;
                    let _ = socket.write_all(response.as_bytes()).await;
                    while socket.read(&mut buffer).await.is_ok_and(|read| read > 0) {}
                });
            }
        });
        (addr, served)
    }

    pub(crate) fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn redirect_to(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    #[flow_like_types::tokio::test]
    async fn redirects_from_the_store_are_not_followed() {
        let (target, reached) = serve(ok("JPEG")).await;
        let (store, _) = serve(redirect_to(&format!("http://{target}/latest/meta-data/"))).await;
        let client = Route::OwnStorage.client().unwrap();
        let fetched = fetch(
            &client,
            url(&format!("http://{store}/content/tmp/user/me/k3x9.jpg")),
            1024,
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(fetched, Err(FetchError::Status(302)));
        assert_eq!(reached.load(Ordering::SeqCst), 0);
    }

    #[flow_like_types::tokio::test]
    async fn guarded_redirects_go_only_where_the_guard_allows() {
        let (target, reached) = serve(ok("JPEG")).await;
        let (hop, _) = serve(redirect_to(&format!(
            "http://cdn.test:{}/k3x9.jpg",
            target.port()
        )))
        .await;
        let builder = Client::builder()
            .resolve("media.test", hop)
            .resolve("cdn.test", target);
        let client = guarded_client(ProviderEgress::Private, builder).unwrap();
        let followed = fetch(
            &client,
            url(&format!("http://media.test:{}/k3x9.jpg", hop.port())),
            1024,
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(followed, Ok(b"JPEG".to_vec()));
        assert_eq!(reached.load(Ordering::SeqCst), 1);

        let (metadata_hop, _) =
            serve(redirect_to("http://169.254.169.254/latest/meta-data/")).await;
        let builder = Client::builder().resolve("media.test", metadata_hop);
        let client = guarded_client(ProviderEgress::Private, builder).unwrap();
        let refused = fetch(
            &client,
            url(&format!(
                "http://media.test:{}/k3x9.jpg",
                metadata_hop.port()
            )),
            1024,
            Duration::from_secs(5),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(refused.contains("169.254.169.254"), "{refused}");
        assert!(!refused.contains("meta-data"), "{refused}");
    }

    #[flow_like_types::tokio::test]
    async fn a_declared_length_over_the_cap_is_refused_unread() {
        let (store, _) =
            serve("HTTP/1.1 200 OK\r\nContent-Length: 104857600\r\n\r\n".to_string()).await;
        let client = Route::OwnStorage.client().unwrap();
        let fetched = fetch(
            &client,
            url(&format!("http://{store}/k3x9.jpg")),
            1024,
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(fetched, Err(FetchError::TooLarge));
    }

    #[flow_like_types::tokio::test]
    async fn a_stalled_body_times_out() {
        let (store, _) =
            serve("HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\npartial".to_string()).await;
        let client = Route::OwnStorage.client().unwrap();
        let fetched = fetch(
            &client,
            url(&format!("http://{store}/k3x9.jpg")),
            1024 * 1024,
            Duration::from_millis(300),
        )
        .await;
        assert_eq!(fetched, Err(FetchError::TimedOut));
    }
}
