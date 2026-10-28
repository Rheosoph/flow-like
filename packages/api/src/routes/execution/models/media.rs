//! The media of a relayed call, resolved before the provider is invoked the way a direct call
//! with the same Bit resolves it on an executor: [`MediaReader::for_provider`] decides, from the
//! Bit's provider, API surface and endpoint, which media the provider fetches from links itself,
//! which it reads only inline and which not at all.
//!
//! A link the provider fetches itself reaches it only when its host is HTTPS on public addresses.
//! A link into the platform's own store is first read with the run's own link, so the store
//! checks the run's access, and then signed again for [`RESIGNED_TTL`]: the provider gets a fresh
//! link to the same object that expires soon, also after the call waited in the hosted worker's
//! queue. Gemini and Vertex get public links like every other provider; their clients decide
//! which ones the provider fetches. Every other link to media the provider reads is resolved
//! here and sent inline, a link into the store as it is and any other under the server egress
//! guard. A link that cannot be resolved fails the call and the error names its host only.

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use flow_like::flow_like_model_provider::{
    egress::ProviderEgress, llm::ModelLogic, provider::ModelProvider,
};
use flow_like::models::media::{
    Link, MediaReader, MediaSource, RemoteMediaModel, publicly_reachable,
};
use flow_like_storage::{Path, files::store::FlowLikeStore};
use flow_like_types::{
    Result, anyhow, bail,
    reqwest::{self, Client, Url, header::RANGE},
    tokio::{sync::OnceCell, time::timeout},
};

use crate::{
    media_links::{FetchError, Route, StorageScope, route},
    state::AppState,
};

/// How long a re-signed link lets the provider fetch its object.
const RESIGNED_TTL: Duration = Duration::from_secs(5 * 60);
/// For the store to answer the run's link.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// `model`, invoked with the media of each call resolved for `provider`.
pub(super) fn resolving(
    model: Arc<dyn ModelLogic>,
    state: &AppState,
    provider: &ModelProvider,
) -> Arc<dyn ModelLogic> {
    let media = RelayMedia::new(Some(state.clone()), OnceCell::new());
    Arc::new(RemoteMediaModel::resolving(
        model,
        Arc::new(media),
        MediaReader::for_provider(provider),
    ))
}

/// The platform's content store, which holds the files of runs.
struct OwnStore {
    store: FlowLikeStore,
    scope: StorageScope,
    /// HTTPS on public addresses only, so a provider can fetch from it.
    reachable: bool,
    /// [`RESIGNED_TTL`], within what the signing credential allows.
    ttl: Duration,
}

impl OwnStore {
    async fn probe(state: &AppState) -> Result<Option<Self>> {
        let credentials = state.master_credentials().await?;
        let store = credentials.to_store(false).await?;
        let Some(scope) = StorageScope::signed_by(&store, "").await? else {
            return Ok(None);
        };
        Ok(Some(Self {
            reachable: publicly_reachable(scope.origin()).await,
            ttl: credentials.signing_ttl(RESIGNED_TTL),
            store,
            scope,
        }))
    }

    /// Reads one byte with the run's own link, so the store decides whether the run may read
    /// the object before it is signed again.
    async fn ensure_readable(&self, client: &Client, url: &Url) -> Result<()> {
        let request = client.get(url.clone()).header(RANGE, "bytes=0-0").send();
        let response = timeout(CHECK_TIMEOUT, request)
            .await
            .map_err(|_| anyhow!("the storage did not answer within {CHECK_TIMEOUT:?}"))?
            .map_err(|error| anyhow!("reading it failed: {}", FetchError::transport(error)))?;
        let status = response.status();
        if !status.is_success() {
            bail!("the storage refused the run's link: HTTP {status}");
        }
        Ok(())
    }

    /// A fresh link to the object `url` names, or `None` when the store would sign another.
    async fn resign(&self, url: &Url) -> Result<Option<String>> {
        let Some(path) = self
            .scope
            .key(url)
            .and_then(|key| Path::from_url_path(key).ok())
        else {
            return Ok(None);
        };
        let fresh = self
            .store
            .sign("GET", &path, self.ttl)
            .await
            .map_err(|error| anyhow!("signing the link again failed: {error}"))?;
        let same_object = fresh.origin() == url.origin() && fresh.path() == url.path();
        Ok(same_object.then(|| fresh.to_string()))
    }
}

/// The relay's downloads, and what the provider receives for each link it fetches itself.
struct RelayMedia {
    /// Probes the own store on first use, unless `own` already holds it.
    state: Option<AppState>,
    own: OnceCell<Option<OwnStore>>,
    egress: ProviderEgress,
    own_client: OnceLock<reqwest::Result<Client>>,
    guarded_client: OnceLock<reqwest::Result<Client>>,
}

impl RelayMedia {
    fn new(state: Option<AppState>, own: OnceCell<Option<OwnStore>>) -> Self {
        Self {
            state,
            own,
            egress: ProviderEgress::for_server(),
            own_client: OnceLock::new(),
            guarded_client: OnceLock::new(),
        }
    }

    async fn own(&self) -> Option<&OwnStore> {
        self.own
            .get_or_init(|| async {
                let state = self.state.as_ref()?;
                OwnStore::probe(state)
                    .await
                    .inspect_err(|error| {
                        tracing::warn!(
                            %error,
                            "The model relay could not sign a storage probe; links into storage are treated as foreign"
                        );
                    })
                    .ok()
                    .flatten()
            })
            .await
            .as_ref()
    }

    fn client(&self, route: Route) -> Result<&Client> {
        let slot = match route {
            Route::OwnStorage => &self.own_client,
            Route::Guarded(_) => &self.guarded_client,
        };
        slot.get_or_init(|| route.client())
            .as_ref()
            .map_err(|error| anyhow!("building the HTTP client for media failed: {error}"))
    }

    fn refusal(&self) -> &'static str {
        match self.egress {
            ProviderEgress::Private => {
                "this server fetches no link to its own network plane (loopback, link-local or metadata)"
            }
            ProviderEgress::Public | ProviderEgress::Open => {
                "this server fetches only public HTTPS links and links into the platform's storage"
            }
        }
    }
}

#[flow_like_types::async_trait]
impl MediaSource for RelayMedia {
    fn cache_scope(&self) -> &str {
        "model-relay"
    }

    async fn prepare(&self) -> Result<()> {
        self.own().await;
        self.client(Route::OwnStorage)?;
        self.client(Route::Guarded(self.egress)).map(|_| ())
    }

    async fn send(&self, url: &str) -> Result<reqwest::Response> {
        let url = Url::parse(url).map_err(|_| anyhow!("the link is not a valid URL"))?;
        let own = self.own().await.map(|own| &own.scope);
        let route = route(&url, own, self.egress).map_err(|_| anyhow!(self.refusal()))?;
        self.client(route)?
            .get(url)
            .send()
            .await
            .map_err(|error| anyhow!("GET failed: {}", FetchError::transport(error)))
    }

    async fn link(&self, url: &str) -> Result<Link> {
        let parsed = Url::parse(url).map_err(|_| anyhow!("the link is not a valid URL"))?;
        if let Some(own) = self.own().await
            && own.scope.contains(&parsed)
        {
            if !own.reachable {
                return Ok(Link::Inline);
            }
            own.ensure_readable(self.client(Route::OwnStorage)?, &parsed)
                .await?;
            return Ok(own
                .resign(&parsed)
                .await?
                .map_or(Link::Inline, Link::Replace));
        }
        Ok(if publicly_reachable(url).await {
            Link::Keep
        } else {
            Link::Inline
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_links::tests::s3_store;
    use flow_like::flow::execution::ExecutionEnvironment;
    use flow_like::flow_like_model_provider::{
        history::{Content, ContentType, History, HistoryMessage, ImageUrl, MessageContent, Role},
        llm::{LLMCallback, ModelConstructor},
        provider::ModelApiSurface,
        response::Response,
    };
    use flow_like_types::tokio::{
        self,
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    use std::{
        net::SocketAddr,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n relay test image";
    const PDF: &[u8] = b"%PDF-1.7 relay test document";
    const DOCX_TYPE: &str =
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    const PUBLIC_IMAGE: &str = "https://93.184.216.34/cat.png";

    /// A media part for a link.
    type Part = fn(&str) -> Content;

    /// What the content bucket on loopback serves by file extension; 403 for anything else.
    fn stored(path: &str) -> Option<(&'static str, &'static [u8])> {
        let extension = path.rsplit_once('.')?.1;
        Some(match extension {
            "png" => ("image/png", PNG),
            "pdf" => ("application/pdf", PDF),
            "docx" => (DOCX_TYPE, b"PK\x03\x04 word/document.xml"),
            "mp3" => ("audio/mpeg", b"ID3 relay audio"),
            "mp4" => ("video/mp4", b"mp4 relay video"),
            "csv" => ("text/csv", b"a,b\n1,2\n"),
            "json" => ("application/json", br#"{"a":1}"#),
            _ => return None,
        })
    }

    /// The content bucket on loopback. Counts requests.
    async fn store_host() -> (SocketAddr, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut buffer = [0u8; 8192];
                    let read = socket.read(&mut buffer).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buffer[..read]).into_owned();
                    let path = head
                        .split_whitespace()
                        .nth(1)
                        .and_then(|target| target.split('?').next())
                        .unwrap_or_default();
                    let (status, content_type, body): (&str, &str, &[u8]) = match stored(path) {
                        Some((content_type, body)) => ("200 OK", content_type, body),
                        None => ("403 Forbidden", "text/plain", b"denied"),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(body).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        (addr, hits)
    }

    /// The store at `addr` as the relay finds it, publicly reachable or not.
    async fn own_store(addr: SocketAddr, reachable: bool) -> OwnStore {
        let store = s3_store(&format!("http://{addr}"));
        let scope = StorageScope::signed_by(&store, "").await.unwrap().unwrap();
        OwnStore {
            store,
            scope,
            reachable,
            ttl: RESIGNED_TTL,
        }
    }

    /// A link into the store the way a run holds one.
    async fn run_link(own: &OwnStore, key: &str) -> String {
        own.store
            .sign("GET", &Path::from(key), Duration::from_secs(3600))
            .await
            .unwrap()
            .to_string()
    }

    fn bit(name: &str) -> ModelProvider {
        ModelProvider {
            provider_name: name.to_string(),
            model_id: None,
            version: None,
            api_surface: None,
            params: None,
        }
    }

    fn chat(name: &str) -> ModelProvider {
        ModelProvider {
            api_surface: Some(ModelApiSurface::ChatCompletions),
            ..bit(name)
        }
    }

    fn image(url: &str) -> Content {
        Content::Image {
            content_type: ContentType::ImageUrl,
            image_url: ImageUrl {
                url: url.to_string(),
                detail: None,
                media_type: None,
                additional_params: None,
            },
        }
    }

    fn document(url: &str, media_type: &str) -> Content {
        Content::Document {
            content_type: ContentType::DocumentUrl,
            document_url: url.to_string(),
            media_type: Some(media_type.to_string()),
            additional_params: None,
        }
    }

    fn pdf(url: &str) -> Content {
        document(url, "application/pdf")
    }

    fn audio(url: &str) -> Content {
        Content::Audio {
            content_type: ContentType::AudioUrl,
            audio_url: url.to_string(),
            media_type: Some("audio/mpeg".to_string()),
            additional_params: None,
        }
    }

    fn video(url: &str) -> Content {
        Content::Video {
            content_type: ContentType::VideoUrl,
            video_url: url.to_string(),
            media_type: Some("video/mp4".to_string()),
            additional_params: None,
        }
    }

    #[derive(Default)]
    struct Recorder {
        histories: Mutex<Vec<History>>,
    }

    #[flow_like_types::async_trait]
    impl ModelLogic for Recorder {
        async fn provider(&self) -> Result<ModelConstructor> {
            Err(anyhow!("the recorder calls no provider"))
        }

        async fn default_model(&self) -> Option<String> {
            None
        }

        async fn invoke(&self, history: &History, _: Option<LLMCallback>) -> Result<Response> {
            self.histories.lock().unwrap().push(history.clone());
            Ok(Response::default())
        }
    }

    /// The media values `model`, which wraps `recorder`, sends its provider for `parts`.
    async fn sent_by(
        model: RemoteMediaModel,
        recorder: &Recorder,
        parts: Vec<Content>,
    ) -> Result<Vec<String>> {
        let mut contents = vec![Content::Text {
            content_type: ContentType::Text,
            text: "What is in these?".to_string(),
        }];
        contents.extend(parts);
        let call = History::new(
            "my-bit".to_string(),
            vec![HistoryMessage {
                role: Role::User,
                content: MessageContent::Contents(contents),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                annotations: None,
            }],
        );

        model.invoke(&call, None).await?;

        let histories = recorder.histories.lock().unwrap();
        let MessageContent::Contents(parts) = &histories[0].messages[0].content else {
            panic!("the call keeps its parts");
        };
        Ok(parts
            .iter()
            .filter_map(Content::media_url)
            .map(ToOwned::to_owned)
            .collect())
    }

    /// The media values the provider of `provider` receives for `parts` through the relay.
    async fn provider_receives(
        own: OwnStore,
        provider: &ModelProvider,
        parts: Vec<Content>,
    ) -> Result<Vec<String>> {
        let media = RelayMedia::new(None, OnceCell::new_with(Some(Some(own))));
        let recorder = Arc::new(Recorder::default());
        let model = RemoteMediaModel::resolving(
            recorder.clone(),
            Arc::new(media),
            MediaReader::for_provider(provider),
        );
        sent_by(model, &recorder, parts).await
    }

    /// The media values the provider of `provider` receives for `parts` from a direct call on
    /// the machine that holds the links.
    async fn direct_call_sends(
        provider: &ModelProvider,
        parts: Vec<Content>,
    ) -> Result<Vec<String>> {
        let recorder = Arc::new(Recorder::default());
        let model = RemoteMediaModel::new(
            recorder.clone(),
            ExecutionEnvironment::Local,
            MediaReader::for_provider(provider),
        );
        sent_by(model, &recorder, parts).await
    }

    fn assert_resigned(sent: &str, held: &str) {
        let (sent, held) = (Url::parse(sent).unwrap(), Url::parse(held).unwrap());
        assert_ne!(sent, held, "the provider gets a fresh link");
        assert_eq!((sent.origin(), sent.path()), (held.origin(), held.path()));
        assert!(
            sent.query_pairs()
                .any(|(name, value)| name == "X-Amz-Expires" && value == "300"),
            "{sent}"
        );
    }

    fn assert_inline(sent: &str, mime: &str, body: &[u8]) {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        assert_eq!(
            sent,
            format!("data:{mime};base64,{}", STANDARD.encode(body))
        );
    }

    #[flow_like_types::tokio::test]
    async fn each_provider_reads_the_links_it_takes_and_gets_the_rest_inline() {
        let (addr, _) = store_host().await;
        for (provider, pdf_sent) in [
            (bit("custom:anthropic"), "inline"),
            (bit("custom:openai"), "re-signed"),
            (chat("custom:openai"), "inline"),
            (bit("custom:openrouter"), "re-signed"),
            (bit("custom:gemini"), "re-signed"),
            (bit("custom:vertex"), "re-signed"),
            (bit("custom:mistral"), "re-signed"),
            (bit("custom:groq"), "left"),
        ] {
            let own = own_store(addr, true).await;
            let cat = run_link(&own, "apps/app-1/storage/cat.png").await;
            let report = run_link(&own, "apps/app-1/storage/report.pdf").await;

            let received = provider_receives(
                own,
                &provider,
                vec![image(&cat), pdf(&report), image(PUBLIC_IMAGE)],
            )
            .await
            .unwrap();

            assert_resigned(&received[0], &cat);
            match pdf_sent {
                "inline" => assert_inline(&received[1], "application/pdf", PDF),
                "re-signed" => assert_resigned(&received[1], &report),
                _ => assert_eq!(received[1], report, "{provider:?}"),
            }
            assert_eq!(received[2], PUBLIC_IMAGE, "{provider:?}");
        }

        let own = own_store(addr, true).await;
        let cat = run_link(&own, "apps/app-1/storage/cat.png").await;
        let received = provider_receives(own, &bit("custom:bedrock"), vec![image(&cat)])
            .await
            .unwrap();
        assert_inline(&received[0], "image/png", PNG);
    }

    #[flow_like_types::tokio::test]
    async fn an_internal_store_is_never_handed_to_the_provider() {
        let (addr, hits) = store_host().await;
        let own = own_store(addr, false).await;
        let cat = run_link(&own, "apps/app-1/storage/cat.png").await;
        let report = run_link(&own, "apps/app-1/storage/report.pdf").await;

        let received = provider_receives(
            own,
            &bit("custom:anthropic"),
            vec![image(&cat), pdf(&report), image(PUBLIC_IMAGE)],
        )
        .await
        .unwrap();

        assert_inline(&received[0], "image/png", PNG);
        assert_inline(&received[1], "application/pdf", PDF);
        assert_eq!(received[2], PUBLIC_IMAGE);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[flow_like_types::tokio::test]
    async fn gemini_and_vertex_keep_public_links_and_get_the_internal_store_inline() {
        let (addr, _) = store_host().await;
        for provider in ["custom:gemini", "custom:vertex"] {
            let own = own_store(addr, false).await;
            let cat = run_link(&own, "apps/app-1/storage/cat.png").await;
            let public = [
                PUBLIC_IMAGE.to_string(),
                "https://93.184.216.34/report.pdf".to_string(),
                "https://93.184.216.34/clip.mp4".to_string(),
            ];

            let received = provider_receives(
                own,
                &bit(provider),
                vec![
                    image(&cat),
                    image(&public[0]),
                    pdf(&public[1]),
                    video(&public[2]),
                ],
            )
            .await
            .unwrap();

            assert_inline(&received[0], "image/png", PNG);
            assert_eq!(received[1..], public, "{provider}");
        }
    }

    #[flow_like_types::tokio::test]
    async fn relayed_and_direct_calls_send_the_same_media() {
        let (addr, _) = store_host().await;
        let providers = [
            bit("custom:anthropic"),
            bit("custom:openai"),
            chat("custom:openai"),
            bit("custom:gemini"),
            bit("custom:vertex"),
            bit("custom:openrouter"),
            bit("custom:mistral"),
            bit("custom:perplexity"),
            bit("custom:deepseek"),
            bit("custom:bedrock"),
            bit("custom:moonshot"),
            bit("custom:ollama"),
            bit("custom:galadriel"),
        ];
        let held: [(&str, Part); 7] = [
            ("cat.png", image),
            ("report.pdf", pdf),
            ("letter.docx", |url| document(url, DOCX_TYPE)),
            ("voice.mp3", audio),
            ("clip.mp4", video),
            ("table.csv", |url| document(url, "text/csv")),
            ("export.json", |url| document(url, "application/json")),
        ];
        for provider in providers {
            let own = own_store(addr, false).await;
            let mut parts = Vec::new();
            for (key, part) in held {
                let link = run_link(&own, &format!("apps/app-1/storage/{key}")).await;
                parts.push(part(&link));
            }

            let relayed = provider_receives(own, &provider, parts.clone())
                .await
                .unwrap();
            let direct = direct_call_sends(&provider, parts.clone()).await.unwrap();

            assert_eq!(relayed, direct, "{provider:?}");
            assert!(
                relayed[5].starts_with("data:text/csv;flow-like-source=string;"),
                "{provider:?}: text documents always travel as their text"
            );
        }
    }

    #[flow_like_types::tokio::test]
    async fn links_outside_the_store_must_be_public_https() {
        let (addr, hits) = store_host().await;
        for (part, host) in [
            (pdf("https://10.0.0.5/private/report.pdf"), "10.0.0.5"),
            (
                image(&format!("http://{addr}/other-bucket/secret.png")),
                "127.0.0.1",
            ),
            (
                image("https://169.254.169.254/latest/meta-data/cat.png"),
                "169.254.169.254",
            ),
        ] {
            let error = provider_receives(
                own_store(addr, false).await,
                &bit("custom:anthropic"),
                vec![part],
            )
            .await
            .unwrap_err()
            .to_string();

            assert!(error.contains(&format!("host '{host}'")), "{error}");
            assert!(error.contains("public HTTPS"), "{error}");
            for path in ["private", "other-bucket", "meta-data"] {
                assert!(!error.contains(path), "{error}");
            }
        }
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[flow_like_types::tokio::test]
    async fn a_link_the_store_refuses_fails_the_call() {
        let (addr, _) = store_host().await;
        let own = own_store(addr, true).await;
        let denied = run_link(&own, "apps/other-app/storage/denied.gif").await;

        let error = provider_receives(own, &bit("custom:anthropic"), vec![image(&denied)])
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("HTTP 403"), "{error}");
        assert!(
            error.contains("image (image/gif) at host '127.0.0.1'"),
            "{error}"
        );
        assert!(!error.contains("other-app"), "{error}");
    }

    #[flow_like_types::tokio::test]
    async fn a_resigned_link_names_the_same_object_and_expires_soon() {
        let (addr, _) = store_host().await;
        let own = own_store(addr, true).await;
        let held = run_link(&own, "apps/app-1/storage/Übersicht (2)#1.png").await;

        let fresh = own
            .resign(&Url::parse(&held).unwrap())
            .await
            .unwrap()
            .expect("the object can be signed again");

        assert_resigned(&fresh, &held);
        let foreign = Url::parse(&format!("http://{addr}/other-bucket/cat.png")).unwrap();
        assert_eq!(own.resign(&foreign).await.unwrap(), None);
    }
}
