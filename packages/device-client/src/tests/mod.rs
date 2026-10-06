mod fake;

use crate::{
    ControllerKeys, DeviceClient, DeviceSession, DeviceTarget, Error, HttpHubClient,
    LoopbackForward, LoopbackPort, LoopbackProxy, TransportKind, TunnelDataOpen, TunnelMode,
    TunnelOpen, TunnelStream, TunnelTarget, hub::AccessToken, hub::HubClient, tls, unix_now,
};
use anyhow::{Context, Result, ensure};
use bytes::Bytes;
use fake::{DeviceOptions, FakeDevice, FakeHub, FakeRelay, RtcMode, Service};
use flow_like_device_crypto::controller::create_controller_vault;
use flow_like_device_protocol::{
    DeviceSignalingResponse, ManagementCommand, ManagementRequest, TUNNEL_INITIAL_WINDOW,
    TUNNEL_MAX_FRAME, TUNNEL_MAX_STREAMS, TunnelEnvelope, TunnelEnvelopeBody, TunnelFrame,
    TunnelFrameBody, TunnelHello, TunnelRenewStart, TunnelRenewed, TunnelReset,
    verify_controller_certificate,
};
use http_body_util::{BodyExt, Full};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Notify, OnceCell},
};
use zeroize::Zeroizing;

const DEVICE: &str = "device-1";
const PASSWORD: &[u8] = b"a long local test password";

/// One Argon2 derivation per process: most tests share these unlocked keys.
async fn shared_keys() -> Arc<ControllerKeys> {
    static KEYS: OnceCell<Arc<ControllerKeys>> = OnceCell::const_new();
    KEYS.get_or_init(|| async { unlock_fresh().await.expect("test controller unlocks") })
        .await
        .clone()
}

async fn vault() -> Vec<u8> {
    static VAULT: OnceCell<Vec<u8>> = OnceCell::const_new();
    VAULT
        .get_or_init(|| async {
            tokio::task::spawn_blocking(|| {
                create_controller_vault(DEVICE, PASSWORD)
                    .expect("test vault")
                    .vault
            })
            .await
            .expect("vault task")
        })
        .await
        .clone()
}

async fn unlock_fresh() -> crate::Result<Arc<ControllerKeys>> {
    ControllerKeys::unlock(DEVICE, Zeroizing::new(PASSWORD.to_vec()), vault().await).await
}

struct Harness {
    hub: Arc<FakeHub>,
    relay: FakeRelay,
    device: Arc<FakeDevice>,
    keys: Arc<ControllerKeys>,
    target: DeviceTarget,
}

impl Harness {
    async fn start(rtc: RtcMode, services: HashMap<String, Service>) -> Result<Self> {
        let keys = shared_keys().await;
        let options = DeviceOptions {
            rtc,
            services,
            ..DeviceOptions::new(keys.controller_key()?)
        };
        Self::with(keys, options, 300).await
    }

    async fn with(
        keys: Arc<ControllerKeys>,
        options: DeviceOptions,
        admission_lifetime: i64,
    ) -> Result<Self> {
        let device = FakeDevice::new(DEVICE, "owner", options);
        let relay = FakeRelay::start(device.clone()).await?;
        let hub = FakeHub::new(&relay.url, 7, admission_lifetime);
        let target = DeviceTarget {
            device_id: DEVICE.into(),
            management_key: device.public,
            auth_epoch: 7,
            grant_id: "owner".into(),
        };
        Ok(Self {
            hub,
            relay,
            device,
            keys,
            target,
        })
    }

    async fn connect(&self) -> crate::Result<DeviceSession> {
        DeviceClient::new(self.hub.clone())
            .connect(self.target.clone(), self.keys.clone())
            .await
    }

    fn assert_clean(&self) {
        let violations = self.device.stats.violations();
        assert!(violations.is_empty(), "{violations:?}");
    }
}

/// A connected harness whose device echoes on service `echo`.
async fn echoing(
    keys: Arc<ControllerKeys>,
    configure: impl FnOnce(DeviceOptions) -> DeviceOptions,
) -> Result<(Harness, DeviceSession)> {
    let options = configure(DeviceOptions {
        services: echo_service().await?,
        ..DeviceOptions::new(keys.controller_key()?)
    });
    let harness = Harness::with(keys, options, 300).await?;
    let session = harness.connect().await?;
    Ok((harness, session))
}

fn services(entries: &[(&str, Service)]) -> HashMap<String, Service> {
    entries
        .iter()
        .map(|(id, service)| ((*id).to_owned(), service.clone()))
        .collect()
}

async fn echo_service() -> Result<HashMap<String, Service>> {
    Ok(services(&[(
        "echo",
        Service::Tcp(fake::echo_server().await?),
    )]))
}

fn service_open(service_id: &str, mode: TunnelMode) -> TunnelOpen {
    TunnelOpen {
        placement_id: "placement".into(),
        service_id: service_id.into(),
        mode,
        target: TunnelTarget::Service,
    }
}

fn gateway_open() -> TunnelOpen {
    TunnelOpen {
        placement_id: String::new(),
        service_id: String::new(),
        mode: TunnelMode::Http,
        target: TunnelTarget::ModelGateway,
    }
}

fn pattern(length: usize, seed: u8) -> Vec<u8> {
    (0..length)
        .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

/// Writes and reads concurrently, so neither side's window ever blocks the other.
async fn echo_through(session: &DeviceSession, bytes: &[u8]) -> Result<()> {
    let stream = session
        .open_stream(service_open("echo", TunnelMode::Tcp))
        .await?;
    let (mut read, mut write) = tokio::io::split(stream);
    let expected = bytes.len();
    let sending = async {
        write.write_all(bytes).await?;
        write.shutdown().await?;
        Ok::<_, anyhow::Error>(write)
    };
    let receiving = async {
        let mut echoed = Vec::with_capacity(expected);
        read.read_to_end(&mut echoed).await?;
        Ok::<_, anyhow::Error>(echoed)
    };
    let (_, echoed) = tokio::try_join!(sending, receiving)?;
    ensure!(echoed == bytes, "the echo changed {} bytes", bytes.len());
    Ok(())
}

async fn echo_bytes(session: &DeviceSession, length: usize) -> Result<()> {
    echo_through(session, &pattern(length, 3)).await
}

/// The browser client (`tunnel-protocol.test.ts`) checks its codec against the same file.
fn fixture() -> Result<serde_json::Value> {
    Ok(serde_json::from_str(include_str!(
        "../../../device-protocol/fixtures/tunnel-v1.json"
    ))?)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn data_frames_match_the_shared_browser_fixture() -> Result<()> {
    let fixture = fixture()?;
    let data = TunnelFrame {
        sequence: fixture["sequence"].as_str().context("sequence")?.parse()?,
        stream_id: u32::try_from(fixture["stream_id"].as_u64().context("stream")?)?,
        body: TunnelFrameBody::Data(vec![0, 1, 254, 255]),
    }
    .encode()?;
    assert_eq!(hex(&data), fixture["data_frame_hex"]);
    Ok(())
}

#[test]
fn envelopes_carry_the_largest_sealed_frame() -> Result<()> {
    let envelope = TunnelEnvelope {
        session_id: "session-abc".into(),
        body: TunnelEnvelopeBody::Message(vec![7; TUNNEL_MAX_FRAME + 16]),
    };
    assert_eq!(TunnelEnvelope::decode(&envelope.encode()?)?, envelope);
    Ok(())
}

#[test]
fn data_opens_match_the_shared_browser_fixture() -> Result<()> {
    let fixture = fixture()?;
    for name in ["open_data_request", "open_data_artifact"] {
        let open: TunnelDataOpen = serde_json::from_value(fixture[name]["body"].clone())?;
        let frame = TunnelFrame {
            sequence: 0x0102_0304_0506_0708,
            stream_id: 0x0102_0305,
            body: TunnelFrameBody::OpenData(open),
        }
        .encode()?;
        assert_eq!(hex(&frame), fixture[name]["frame_hex"], "{name}");
        assert_eq!(TunnelFrame::decode(&frame)?.encode()?, frame);
    }
    Ok(())
}

/// The browser validates these JSON bodies with exact key sets (`tunnel-protocol.ts`).
#[test]
fn json_bodies_carry_exactly_the_keys_the_browser_expects() -> Result<()> {
    let keys = |value: serde_json::Value| -> Vec<String> {
        let mut keys: Vec<String> = value
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        keys
    };
    assert_eq!(
        serde_json::to_value(gateway_open())?,
        serde_json::json!({"mode":"http","target":"model_gateway"})
    );
    assert_eq!(
        keys(serde_json::to_value(service_open(
            "hosting",
            TunnelMode::Http
        ))?),
        ["mode", "placement_id", "service_id"]
    );
    assert_eq!(
        keys(serde_json::to_value(TunnelRenewed { expires_at: 9 })?),
        ["expires_at"]
    );
    assert_eq!(
        keys(serde_json::to_value(TunnelReset {
            code: "cancelled".into(),
            message: "m".into()
        })?),
        ["code", "message"]
    );
    assert_eq!(
        keys(serde_json::to_value(TunnelRenewStart {
            certificate_jws: "c".into(),
            data: "AA".into()
        })?),
        ["certificate_jws", "data"]
    );
    assert_eq!(
        keys(serde_json::to_value(TunnelHello {
            grant_id: "owner".into(),
            certificate_jws: "c".into(),
            data: "AA".into()
        })?),
        ["certificate_jws", "data", "grant_id"]
    );
    Ok(())
}

#[test]
fn tls_config_builds_while_both_rustls_providers_are_linked() -> Result<()> {
    let config = tls::client_config()?;
    assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);
    assert!(Arc::ptr_eq(&config, &tls::client_config()?));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keys_unlock_certify_short_sessions_and_zeroize_on_lock() -> Result<()> {
    let wrong = ControllerKeys::unlock(
        DEVICE,
        Zeroizing::new(b"not the password".to_vec()),
        vault().await,
    )
    .await;
    assert!(
        matches!(wrong, Err(Error::Unlock { .. })),
        "{:?}",
        wrong.err()
    );
    let other =
        ControllerKeys::unlock("device-2", Zeroizing::new(PASSWORD.to_vec()), vault().await).await;
    assert!(matches!(other, Err(Error::Unlock { .. })));
    let keys = unlock_fresh().await?;
    let device_key = x25519_dalek::x25519([9; 32], x25519_dalek::X25519_BASEPOINT_BYTES);
    let now = unix_now();
    let certified = keys.begin_tunnel("owner", device_key, now)?;
    assert_eq!(
        certified.certificate.expires_at - certified.certificate.issued_at,
        300
    );
    let verified =
        verify_controller_certificate(&certified.certificate_jws, &keys.controller_key()?, now)?;
    assert_eq!(verified, certified.certificate);
    let renewed = keys.begin_tunnel("owner", device_key, now)?;
    assert_ne!(
        renewed.certificate.session_id,
        certified.certificate.session_id
    );
    assert_ne!(
        renewed.certificate.management_key,
        certified.certificate.management_key
    );
    keys.lock();
    assert!(keys.is_locked());
    assert!(matches!(
        keys.begin_tunnel("owner", device_key, now),
        Err(Error::Locked { .. })
    ));
    assert!(matches!(keys.controller_key(), Err(Error::Locked { .. })));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handshake_verifies_the_certificate_and_the_device_key() -> Result<()> {
    let harness = Harness::start(RtcMode::Mismatch, echo_service().await?).await?;
    let session = harness.connect().await?;
    assert_eq!(session.transport(), Some(TransportKind::Relay));
    assert_eq!(harness.device.stats.hellos.load(Ordering::SeqCst), 1);
    echo_bytes(&session, 1000).await?;

    let mut impostor = harness.target.clone();
    impostor.management_key = x25519_dalek::x25519([5; 32], x25519_dalek::X25519_BASEPOINT_BYTES);
    let error = DeviceClient::new(harness.hub.clone())
        .connect(impostor, harness.keys.clone())
        .await
        .err()
        .context("a device with another Noise key was accepted")?;
    assert!(error.to_string().contains("pinned identity"), "{error}");
    harness.assert_clean();

    let stranger = flow_like_device_protocol::SigningKey::generate().public_key();
    let refusing = Harness::with(harness.keys.clone(), DeviceOptions::new(stranger), 300).await?;
    let error = refusing
        .connect()
        .await
        .err()
        .context("a device that distrusts this controller admitted it")?;
    assert!(error.to_string().contains("handshake"), "{error}");
    assert_eq!(refusing.device.stats.hellos.load(Ordering::SeqCst), 0);
    assert!(refusing.device.stats.refusals.load(Ordering::SeqCst) >= 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webrtc_is_preferred_and_carries_streams() -> Result<()> {
    let harness = Harness::start(RtcMode::Answer, echo_service().await?).await?;
    let session = harness.connect().await?;
    assert_eq!(session.transport(), Some(TransportKind::WebRtc));
    echo_bytes(&session, 600 * 1024).await?;
    harness.assert_clean();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_fallback_carries_concurrent_streams_within_their_windows() -> Result<()> {
    let harness = Harness::start(RtcMode::Mismatch, echo_service().await?).await?;
    let session = harness.connect().await?;
    assert_eq!(session.transport(), Some(TransportKind::Relay));
    let transfers = (0..6u8).map(|seed| {
        let session = session.clone();
        async move { echo_through(&session, &pattern(700 * 1024 + usize::from(seed), seed)).await }
    });
    let finished = tokio::time::timeout(
        Duration::from_secs(30),
        futures_util::future::try_join_all(transfers),
    )
    .await;
    harness.assert_clean();
    finished??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writes_stop_at_the_device_window_until_it_consumes() -> Result<()> {
    let harness = Harness::start(RtcMode::Mismatch, services(&[("stall", Service::Stall)])).await?;
    let session = harness.connect().await?;
    let mut stream = session
        .open_stream(service_open("stall", TunnelMode::Tcp))
        .await?;
    let chunk = vec![7u8; 64 * 1024];
    let mut accepted = 0usize;
    while let Ok(result) =
        tokio::time::timeout(Duration::from_millis(500), stream.write(&chunk)).await
    {
        accepted += result?;
    }
    assert_eq!(accepted, TUNNEL_INITIAL_WINDOW as usize);
    tokio::time::timeout(Duration::from_secs(5), async {
        while harness.device.stats.stalled_bytes.load(Ordering::SeqCst) < accepted {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    harness.assert_clean();
    Ok(())
}

/// The device reports a 62 s lifetime, so the controller renews after about 2 s while
/// device heartbeats keep arriving.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn certificate_renewal_keeps_streams_open() -> Result<()> {
    let (harness, session) = echoing(shared_keys().await, |options| DeviceOptions {
        lifetime: 62,
        heartbeat: Duration::from_millis(100),
        ..options
    })
    .await?;
    let stream = session
        .open_stream(service_open("echo", TunnelMode::Tcp))
        .await?;
    let renewed = || harness.device.stats.renewals.load(Ordering::SeqCst) > 0;
    echo_rounds_until(stream, |elapsed| {
        elapsed >= Duration::from_secs(5) && renewed()
    })
    .await?;
    assert!(harness.device.stats.pongs.load(Ordering::SeqCst) >= 1);
    echo_bytes(&session, 64 * 1024).await?;
    assert_eq!(harness.device.stats.hellos.load(Ordering::SeqCst), 1);
    harness.assert_clean();
    Ok(())
}

/// Echoes small rounds on one stream until `done` holds, for at most 20 s.
async fn echo_rounds_until(stream: TunnelStream, done: impl Fn(Duration) -> bool) -> Result<()> {
    let (mut read, mut write) = tokio::io::split(stream);
    let started = tokio::time::Instant::now();
    let mut round = 0u8;
    while !done(started.elapsed()) {
        ensure!(
            started.elapsed() < Duration::from_secs(20),
            "the condition never held"
        );
        let sent = pattern(4096, round);
        write.write_all(&sent).await?;
        let mut echoed = vec![0; sent.len()];
        read.read_exact(&mut echoed).await?;
        ensure!(echoed == sent, "round {round} echoed other bytes");
        round = round.wrapping_add(1);
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Ok(())
}

/// The hub admits the relay for 62 s, so the client re-authorizes the same participant
/// in place after about 2 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_admission_renews_in_place() -> Result<()> {
    let keys = shared_keys().await;
    let options = DeviceOptions {
        services: echo_service().await?,
        ..DeviceOptions::new(keys.controller_key()?)
    };
    let harness = Harness::with(keys, options, 62).await?;
    let session = harness.connect().await?;
    tokio::time::timeout(Duration::from_secs(15), async {
        while harness.relay.reauthorized.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    assert_eq!(harness.hub.admissions.load(Ordering::SeqCst), 2);
    echo_bytes(&session, 10_000).await?;
    assert_eq!(harness.device.stats.hellos.load(Ordering::SeqCst), 1);
    harness.assert_clean();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_streams_carry_bulk_reads() -> Result<()> {
    let harness = Harness::start(RtcMode::Mismatch, HashMap::new()).await?;
    let session = harness.connect().await?;
    let now = unix_now();
    let mut stream = session
        .open_data(TunnelDataOpen::Request {
            request: ManagementRequest {
                operation_id: "read-logs".into(),
                device_id: DEVICE.into(),
                issued_at: now,
                expires_at: now + 60,
                command: ManagementCommand::Logs {
                    placement_id: Some("placement".into()),
                    after: 0,
                    limit: 10,
                },
            },
        })
        .await?;
    stream.shutdown().await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let response: serde_json::Value = serde_json::from_slice(&response)?;
    assert_eq!(response["operation_id"], "read-logs");
    assert_eq!(response["result"]["device_id"], DEVICE);
    let refused = session
        .open_stream(service_open("missing", TunnelMode::Tcp))
        .await;
    assert!(
        matches!(&refused, Err(Error::Reset { code, .. }) if code == "unauthorized"),
        "{refused:?}"
    );
    harness.assert_clean();
    Ok(())
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .use_preconfigured_tls((*tls::client_config()?).clone())
        .build()?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proxy_streams_events_and_requires_its_bearer() -> Result<()> {
    let release = Arc::new(Notify::new());
    let (harness, proxy) = sse_proxy(release.clone()).await?;
    let client = http_client()?;
    let url = format!("{}/v1/events", proxy.base_url());
    assert_bearer_required(&client, &url).await?;
    assert_eq!(harness.device.stats.opens.load(Ordering::SeqCst), 0);
    for _ in 0..2 {
        let response = client.get(&url).bearer_auth(proxy.bearer()).send().await?;
        assert_forwarded(&response);
        let (first, rest) = read_events(response, &release).await?;
        assert_eq!(first, b"data: one /v1/events\n\n");
        assert_eq!(rest, b"data: two\n\n");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        harness.device.stats.opens.load(Ordering::SeqCst),
        1,
        "the pooled stream was not reused"
    );
    proxy.close();
    harness.assert_clean();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proxy_counts_a_streamed_answer_as_in_flight_until_its_body_ends() -> Result<()> {
    let release = Arc::new(Notify::new());
    let (harness, proxy) = sse_proxy(release.clone()).await?;
    let client = http_client()?;
    let url = format!("{}/v1/events", proxy.base_url());
    assert_bearer_required(&client, &url).await?;
    assert_eq!(proxy.activity(), Default::default(), "refusals are no use");

    let response = client.get(&url).bearer_auth(proxy.bearer()).send().await?;
    let started = proxy.activity();
    assert_eq!(started.in_flight, 1);
    let first_seen = started.last_request.context("the request was stamped")?;
    let (_, rest) = read_events(response, &release).await?;
    assert_eq!(rest, b"data: two\n\n");
    let ended = settled(&proxy).await?;
    assert!(ended.last_request.is_some_and(|last| last > first_seen));
    proxy.close();
    harness.assert_clean();
    Ok(())
}

/// The proxy's activity once no request is in flight any more.
async fn settled(proxy: &LoopbackProxy) -> Result<crate::ProxyActivity> {
    let wait = async {
        loop {
            let activity = proxy.activity();
            if activity.in_flight == 0 {
                return activity;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    Ok(tokio::time::timeout(Duration::from_secs(5), wait).await?)
}

async fn sse_proxy(release: Arc<Notify>) -> Result<(Harness, LoopbackProxy)> {
    let sse = fake::sse_server(release).await?;
    let gateway = services(&[("model_gateway", Service::Tcp(sse))]);
    let harness = Harness::start(RtcMode::Mismatch, gateway).await?;
    let session = harness.connect().await?;
    let proxy = LoopbackPort::bind("localhost")
        .await?
        .serve(session, gateway_open())?;
    Ok((harness, proxy))
}

async fn assert_bearer_required(client: &reqwest::Client, url: &str) -> Result<()> {
    assert_eq!(client.get(url).send().await?.status(), 401);
    let wrong = client.get(url).bearer_auth("not-the-token").send().await?;
    assert_eq!(wrong.status(), 401);
    assert_eq!(wrong.headers()["www-authenticate"], "Bearer");
    Ok(())
}

/// The service saw the configured host and no bearer, and its headers came back intact.
fn assert_forwarded(response: &reqwest::Response) {
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["x-saw-authorization"], "false");
    assert_eq!(response.headers()["x-host"], "localhost");
    assert_eq!(response.headers()["content-type"], "text/event-stream");
}

/// Returns the first event as soon as it arrives, then releases the second and reads to
/// the end. The first arriving alone proves the proxy does not buffer.
async fn read_events(
    mut response: reqwest::Response,
    release: &Notify,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let wait = Duration::from_secs(5);
    let mut first = Vec::new();
    while !first.ends_with(b"\n\n") {
        let chunk = tokio::time::timeout(wait, response.chunk()).await??;
        first.extend_from_slice(&chunk.context("the stream ended before the first event")?);
    }
    release.notify_one();
    let mut rest = Vec::new();
    while let Some(chunk) = tokio::time::timeout(wait, response.chunk()).await?? {
        rest.extend_from_slice(&chunk);
    }
    Ok((first, rest))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proxy_reports_a_refused_gateway_as_bad_gateway() -> Result<()> {
    let harness = Harness::start(RtcMode::Mismatch, HashMap::new()).await?;
    let session = harness.connect().await?;
    let proxy = LoopbackPort::bind("localhost")
        .await?
        .serve(session, gateway_open())?;
    let response = http_client()?
        .post(format!("{}/v1/chat/completions", proxy.base_url()))
        .bearer_auth(proxy.bearer())
        .body("{}")
        .send()
        .await?;
    assert_eq!(response.status(), 502);
    let body = response.text().await?;
    assert!(body.contains("unauthorized"), "{body}");
    harness.assert_clean();
    Ok(())
}

/// A device whose model gateway streams events, and a loopback port for its proxies.
struct EventsPort {
    harness: Harness,
    session: DeviceSession,
    port: Arc<LoopbackPort>,
    release: Arc<Notify>,
    client: reqwest::Client,
}

impl EventsPort {
    async fn start() -> Result<Self> {
        let release = Arc::new(Notify::new());
        let sse = fake::sse_server(release.clone()).await?;
        let gateway = services(&[("model_gateway", Service::Tcp(sse))]);
        let harness = Harness::start(RtcMode::Mismatch, gateway).await?;
        Ok(Self {
            session: harness.connect().await?,
            harness,
            port: LoopbackPort::bind("localhost").await?,
            release,
            client: http_client()?,
        })
    }

    fn url(&self) -> String {
        format!("http://{}/v1/events", self.port.local_addr())
    }

    fn serve(&self) -> crate::Result<LoopbackProxy> {
        self.port.serve(self.session.clone(), gateway_open())
    }

    async fn status(&self, bearer: &str) -> Result<u16> {
        let response = self
            .client
            .get(self.url())
            .bearer_auth(bearer)
            .send()
            .await?;
        Ok(response.status().as_u16())
    }

    /// The first event, once both arrived through the proxy of `bearer`.
    async fn events(&self, bearer: &str) -> Result<Vec<u8>> {
        let response = self
            .client
            .get(self.url())
            .bearer_auth(bearer)
            .send()
            .await?;
        assert_forwarded(&response);
        Ok(read_events(response, &self.release).await?.0)
    }
}

/// A client that kept the address of a closed proxy reaches no other process: the port
/// stays bound and answers 503.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_proxy_keeps_its_port_and_answers_503() -> Result<()> {
    let device = EventsPort::start().await?;
    let proxy = device.serve()?;
    let stale = proxy.bearer().to_owned();
    assert_eq!(device.events(&stale).await?, b"data: one /v1/events\n\n");

    drop(proxy);
    assert_eq!(device.status(&stale).await?, 503);
    assert!(
        std::net::TcpListener::bind(device.port.local_addr()).is_err(),
        "no other process can take the port"
    );
    device.harness.assert_clean();
    Ok(())
}

/// The next proxy on a port has a bearer of its own, so an old client never reaches the
/// later session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_later_proxy_on_the_port_refuses_an_earlier_proxys_bearer() -> Result<()> {
    let device = EventsPort::start().await?;
    let stale = device.serve()?.bearer().to_owned();
    let later = device.serve()?;

    assert_eq!(
        later.base_url(),
        format!("http://{}", device.port.local_addr())
    );
    assert_eq!(device.status(&stale).await?, 401);
    assert_eq!(
        device.events(later.bearer()).await?,
        b"data: one /v1/events\n\n"
    );
    device.harness.assert_clean();
    Ok(())
}

/// A gateway that answers once `parallel` requests are waiting, so a burst really runs on
/// that many streams at once.
async fn gathering_gateway(parallel: usize) -> Result<std::net::SocketAddr> {
    let gathered = Arc::new(tokio::sync::Barrier::new(parallel));
    fake::http_server(move |request: hyper::Request<hyper::body::Incoming>| {
        let gathered = gathered.clone();
        async move {
            let _ = request.into_body().collect().await;
            gathered.wait().await;
            hyper::Response::new(Full::new(Bytes::from_static(b"ok")).boxed())
        }
    })
    .await
}

/// A device whose model gateway answers `parallel` requests at once, and an echo service.
async fn gathering_device(parallel: usize) -> Result<(Harness, DeviceSession)> {
    let gateway = Service::Tcp(gathering_gateway(parallel).await?);
    let echo = Service::Tcp(fake::echo_server().await?);
    let harness = Harness::start(
        RtcMode::Mismatch,
        services(&[("model_gateway", gateway), ("echo", echo)]),
    )
    .await?;
    let session = harness.connect().await?;
    Ok((harness, session))
}

/// The answers to `count` requests sent through `proxy` at once.
async fn burst(proxy: &LoopbackProxy, count: usize) -> Result<Vec<String>> {
    let client = http_client()?;
    let url = format!("{}/v1/models", proxy.base_url());
    let requests = (0..count).map(|_| {
        let request = client.get(&url).bearer_auth(proxy.bearer());
        async move { request.send().await?.text().await }
    });
    let all = futures_util::future::try_join_all(requests);
    Ok(tokio::time::timeout(Duration::from_secs(10), all).await??)
}

/// One tunnel carries model calls, local ports and transfers. After a burst of model calls,
/// the proxy's idle connections keep at most two of its streams, so the other opens start at
/// once instead of waiting for a slot until they time out.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_of_proxied_requests_leaves_the_tunnel_room_for_other_streams() -> Result<()> {
    const BURST: usize = 8;
    const KEPT_IDLE: usize = 2;
    let (harness, session) = gathering_device(BURST).await?;
    let port = LoopbackPort::bind("localhost").await?;
    let proxy = port.serve(session.clone(), gateway_open())?;
    assert_eq!(burst(&proxy, BURST).await?, vec!["ok"; BURST]);

    let free = TUNNEL_MAX_STREAMS - KEPT_IDLE;
    let opens = (0..free).map(|_| session.open_stream(service_open("echo", TunnelMode::Tcp)));
    let streams = tokio::time::timeout(
        Duration::from_secs(5),
        futures_util::future::try_join_all(opens),
    )
    .await
    .context("the proxy's idle connections held the tunnel's streams")??;
    assert_eq!(streams.len(), free);
    harness.assert_clean();
    Ok(())
}

#[test]
fn proxies_accept_only_origin_form_targets() {
    use crate::proxy::origin_form;
    let uri = |value: &str| value.parse::<http::Uri>().expect("test URI");
    assert_eq!(origin_form(&uri("/v1/models?x=1")), Some("/v1/models?x=1"));
    assert_eq!(origin_form(&uri("http://elsewhere/v1/models")), None);
    assert_eq!(origin_form(&uri("elsewhere:443")), None);
    assert_eq!(origin_form(&uri("*")), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forward_carries_raw_tcp() -> Result<()> {
    let harness = Harness::start(RtcMode::Mismatch, echo_service().await?).await?;
    let session = harness.connect().await?;
    let forward = LoopbackForward::bind(session, service_open("echo", TunnelMode::Tcp), 0).await?;
    let socket = tokio::net::TcpStream::connect(forward.local_addr()).await?;
    let sent = pattern(200 * 1024, 11);
    assert_eq!(round_trip(socket, &sent).await?, sent);
    harness.assert_clean();
    Ok(())
}

async fn round_trip(mut socket: tokio::net::TcpStream, bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let (mut read, mut write) = socket.split();
    let sending = async {
        write.write_all(bytes).await?;
        write.shutdown().await
    };
    let mut echoed = Vec::new();
    tokio::try_join!(sending, read.read_to_end(&mut echoed))?;
    Ok(echoed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_reconnects_after_the_transport_drops() -> Result<()> {
    let harness = Harness::start(RtcMode::Mismatch, echo_service().await?).await?;
    let session = harness.connect().await?;
    let mut stream = session
        .open_stream(service_open("echo", TunnelMode::Tcp))
        .await?;
    let tunnel = session.live_tunnel().context("no live tunnel")?;
    harness.relay.drop_connections();
    let ended = tokio::time::timeout(Duration::from_secs(10), tunnel.closed()).await?;
    assert!(
        matches!(ended, Error::Closed { .. } | Error::Unreachable { .. }),
        "{ended:?}"
    );
    let mut buffer = [0; 8];
    assert!(stream.read(&mut buffer).await.is_err());
    echo_bytes(&session, 10_000).await?;
    assert_eq!(harness.hub.admissions.load(Ordering::SeqCst), 2);
    assert_eq!(harness.device.stats.hellos.load(Ordering::SeqCst), 2);
    harness.assert_clean();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn locking_the_keys_closes_the_tunnel() -> Result<()> {
    let keys = unlock_fresh().await?;
    let (_harness, session) = echoing(keys.clone(), |options| options).await?;
    let mut stream = session
        .open_stream(service_open("echo", TunnelMode::Tcp))
        .await?;
    keys.lock();
    let mut buffer = [0; 8];
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer)).await?;
    let failure = read.err().context("a stream outlived the lock")?;
    let cause = failure.get_ref().and_then(|inner| inner.downcast_ref());
    assert!(matches!(cause, Some(Error::Locked { .. })), "{failure:?}");
    let reopened = session
        .open_stream(service_open("echo", TunnelMode::Tcp))
        .await;
    assert!(
        matches!(reopened, Err(Error::Locked { .. })),
        "{reopened:?}"
    );
    Ok(())
}

struct StaticToken;

#[async_trait::async_trait]
impl AccessToken for StaticToken {
    async fn access_token(&self) -> crate::Result<String> {
        Ok("test-token".into())
    }
}

/// Admits only `participant-1` to `device-1` with the test token, like the API route.
async fn admission_endpoint(
    request: hyper::Request<hyper::body::Incoming>,
) -> hyper::Response<fake::BoxedBody> {
    let expected = request.uri().path() == "/api/v1/devices/device-1/signaling/controller"
        && request
            .headers()
            .get("authorization")
            .is_some_and(|value| value == "Bearer test-token");
    let body = request
        .into_body()
        .collect()
        .await
        .map(|body| body.to_bytes());
    let participant =
        serde_json::from_slice::<serde_json::Value>(&body.unwrap_or_default()).unwrap_or_default();
    let admitted = expected && participant == serde_json::json!({"participant_id":"participant-1"});
    let admission = DeviceSignalingResponse {
        token: "token".into(),
        expires_at: unix_now() + 300,
        device_auth_epoch: 7,
        signaling_urls: vec!["wss://relay.example/ws/devices".into()],
        ice_servers: Vec::new(),
        ice_expires_at: None,
        policy_version: 0,
        policy_digest: None,
    };
    let (status, json) = if admitted {
        (200, serde_json::to_vec(&admission).unwrap_or_default())
    } else {
        (403, br#"{"error":"Forbidden"}"#.to_vec())
    };
    let body = http_body_util::Full::new(bytes::Bytes::from(json))
        .map_err(|never| match never {})
        .boxed();
    hyper::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(body)
        .expect("static response")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http_hub_client_posts_the_participant_and_reports_refusals() -> Result<()> {
    let address = fake::http_server(admission_endpoint).await?;
    let hub = HttpHubClient::new(&format!("http://{address}/api/v1"), Arc::new(StaticToken))?;
    let admission = hub.controller_admission(DEVICE, "participant-1").await?;
    assert_eq!(admission.device_auth_epoch, 7);
    let refused = hub.controller_admission("device-2", "participant-1").await;
    assert!(
        matches!(&refused, Err(Error::Refused { status: Some(403), message, .. }) if message.contains("Forbidden")),
        "{refused:?}"
    );
    assert!(HttpHubClient::new("http://hub.example/api/v1", Arc::new(StaticToken)).is_err());
    assert!(HttpHubClient::new("https://hub.example/api/v1?x=1", Arc::new(StaticToken)).is_err());
    Ok(())
}

#[test]
fn admissions_bind_the_receipt_epoch_and_a_short_lifetime() {
    let target = DeviceTarget {
        device_id: DEVICE.into(),
        management_key: x25519_dalek::x25519([9; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
        auth_epoch: 7,
        grant_id: "owner".into(),
    };
    let now = 1_000_000;
    let admission = |epoch: u64, expires_at: i64, urls: usize| DeviceSignalingResponse {
        token: "token".into(),
        expires_at,
        device_auth_epoch: epoch,
        signaling_urls: vec!["wss://relay.example/ws/devices".into(); urls],
        ice_servers: Vec::new(),
        ice_expires_at: None,
        policy_version: 0,
        policy_digest: None,
    };
    use crate::client::validate_admission;
    assert!(validate_admission(&admission(7, now + 300, 1), &target, now).is_ok());
    for (epoch, expires_at, urls) in [
        (8, now + 300, 1),
        (7, now + 5, 1),
        (7, now + 900, 1),
        (7, now + 300, 0),
        (7, now + 300, 5),
    ] {
        assert!(matches!(
            validate_admission(&admission(epoch, expires_at, urls), &target, now),
            Err(Error::Refused { .. })
        ));
    }
}
