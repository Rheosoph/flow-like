use super::{
    AUTH_CHECK, PollFailure, Session, lock,
    pacing::{BACKOFF_CAP, backoff, jitter},
};
use flow_like_types::realtime::{
    RealtimeConnectRequest, RealtimeConnectResponse, RealtimeTransport,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Notify, watch};

const MAX_GRANT_BYTES: usize = 64 * 1024;
const RENEWAL_MARGIN: Duration = Duration::from_secs(20);
const RECONNECT: Duration = Duration::from_secs(2);
const BLOCKED: Duration = Duration::from_secs(30);
/// Each grant costs the API a provider credential (an STS AssumeRole on AWS), so only a
/// connection that stayed up this long stops reconnect delays from compounding.
const STABLE: Duration = Duration::from_secs(60);

pub(super) struct ProviderConnection {
    pub wake: Arc<Notify>,
    healthy: watch::Receiver<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl ProviderConnection {
    /// `reconnect` outlives the connection, so restarting it after a blocked poll keeps the
    /// grant backoff instead of granting again at once.
    pub fn start(
        client: reqwest::Client,
        session: Arc<Session>,
        max_wait: Arc<AtomicU64>,
        reconnect: Arc<Mutex<Reconnect>>,
    ) -> Self {
        let wake = Arc::new(Notify::new());
        let (sender, healthy) = watch::channel(false);
        let notifications = Arc::clone(&wake);
        let task = tokio::spawn(async move {
            let work = run(
                &client,
                &session,
                notifications,
                sender.clone(),
                &max_wait,
                &reconnect,
            );
            tokio::pin!(work);
            loop {
                tokio::select! {
                    _ = &mut work => break,
                    _ = tokio::time::sleep(AUTH_CHECK) => {
                        if !session.binding.is_current() { break; }
                    }
                }
            }
            sender.send_replace(false);
        });
        Self {
            wake,
            healthy,
            task,
        }
    }

    pub fn is_healthy(&self) -> bool {
        *self.healthy.borrow()
    }

    pub async fn changed(&mut self) {
        if self.healthy.changed().await.is_err() {
            // A signed-out provider leaves the ordinary HTTP owner running.
            std::future::pending::<()>().await;
        }
    }
}

impl Drop for ProviderConnection {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Seconds the server granted, measured from receipt so a skewed local clock cannot expire it.
fn lifetime(grant: &RealtimeConnectResponse, now: i64) -> Option<Duration> {
    let seconds = match grant.expires_in {
        Some(seconds) => seconds,
        None => u64::try_from(grant.expires_at.saturating_sub(now)).ok()?,
    };
    (1..=86_400)
        .contains(&seconds)
        .then(|| Duration::from_secs(seconds))
}

async fn grant(
    client: &reqwest::Client,
    session: &Session,
) -> Result<(RealtimeConnectResponse, Instant), PollFailure> {
    let body = super::post(
        client,
        session,
        "connect",
        &RealtimeConnectRequest::default(),
        MAX_GRANT_BYTES,
    )
    .await?;
    let received = Instant::now();
    let grant = super::decode(&body, |grant: &RealtimeConnectResponse| {
        grant.protocol_version
    })?;
    let lifetime =
        lifetime(&grant, chrono::Utc::now().timestamp()).ok_or(PollFailure::Unsupported)?;
    Ok((grant, received + lifetime))
}

fn loopback(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}

fn encrypted(url: &str, secure: &str, cleartext: &str) -> bool {
    reqwest::Url::parse(url)
        .is_ok_and(|url| url.scheme() == secure || (url.scheme() == cleartext && loopback(&url)))
}

/// An HTTPS API never hands account wakeup credentials to a cleartext provider unless the
/// provider is on loopback. NATS without a TLS scheme is upgraded so the client requires TLS.
fn secure_transport(hub: &str, transport: RealtimeTransport) -> Option<RealtimeTransport> {
    if reqwest::Url::parse(hub).ok()?.scheme() != "https" {
        return Some(transport);
    }
    match transport {
        RealtimeTransport::Nats {
            url,
            subject,
            token,
        } => {
            let mut parsed = reqwest::Url::parse(&url).ok()?;
            let url = match parsed.scheme() {
                "tls" | "wss" => url,
                "nats" | "ws" if loopback(&parsed) => url,
                "nats" => {
                    parsed.set_scheme("tls").ok()?;
                    parsed.into()
                }
                _ => return None,
            };
            Some(RealtimeTransport::Nats {
                url,
                subject,
                token,
            })
        }
        RealtimeTransport::AzureWebPubSub { ref url, .. } => {
            encrypted(url, "wss", "ws").then_some(transport)
        }
        RealtimeTransport::GcpFirebaseRtdb {
            ref database_url, ..
        } => encrypted(database_url, "https", "http").then_some(transport),
        RealtimeTransport::Http {} | RealtimeTransport::AwsMqtt { .. } => Some(transport),
    }
}

#[derive(Default)]
pub(super) struct Reconnect {
    failures: u32,
    not_before: Option<Instant>,
}

impl Reconnect {
    fn wait(&self) -> Duration {
        self.not_before.map_or(Duration::ZERO, |at| {
            at.saturating_duration_since(Instant::now())
        })
    }

    fn failed(&mut self, base: Duration) -> Duration {
        self.failures = self.failures.saturating_add(1);
        jitter(backoff(base, BACKOFF_CAP, self.failures))
    }

    /// `None` renews at once: the listener stayed connected until its grant needed renewal.
    fn ended(&mut self, renewed: bool, uptime: Option<Duration>) -> Option<Duration> {
        if uptime.is_some_and(|uptime| uptime >= STABLE) {
            self.failures = 0;
        }
        if renewed && uptime.is_some() {
            return None;
        }
        Some(self.failed(RECONNECT))
    }
}

/// Listens until the grant needs renewal. Returns whether renewal ended it and how long the
/// provider connection stayed up.
async fn listen(
    transport: &RealtimeTransport,
    wake: &Arc<Notify>,
    healthy: &watch::Sender<bool>,
    renew: Instant,
) -> (bool, Option<Duration>) {
    let mut up = healthy.subscribe();
    let listener =
        flow_like_channels::realtime::listen(transport, Arc::clone(wake), healthy.clone());
    tokio::pin!(listener);
    let deadline = tokio::time::sleep_until(renew.into());
    tokio::pin!(deadline);
    let mut connected = None;
    let renewed = loop {
        tokio::select! {
            _ = &mut listener => break false,
            _ = &mut deadline => break true,
            Ok(()) = up.changed(), if connected.is_none() => {
                if *up.borrow_and_update() {
                    connected = Some(Instant::now());
                }
            }
        }
    };
    (renewed, connected.map(|at: Instant| at.elapsed()))
}

/// Uses one grant until it needs renewal or fails. Returns the delay before the next grant.
async fn serve(
    hub: &str,
    transport: RealtimeTransport,
    expires: Instant,
    wake: &Arc<Notify>,
    healthy: &watch::Sender<bool>,
    reconnect: &Mutex<Reconnect>,
) -> Duration {
    let remaining = expires.saturating_duration_since(Instant::now());
    let Some(transport) = secure_transport(hub, transport) else {
        tracing::warn!(
            "Refused a cleartext realtime provider grant from an HTTPS API; polling over HTTP"
        );
        return lock(reconnect).failed(RECONNECT);
    };
    if matches!(transport, RealtimeTransport::Http {}) {
        // Keep a configured HTTP deployment inexpensive while allowing a
        // later provider configuration to take effect without restarting.
        return remaining.clamp(Duration::from_secs(30), Duration::from_secs(900));
    }
    let renew = Instant::now()
        + remaining
            .saturating_sub(RENEWAL_MARGIN)
            .max(Duration::from_secs(1));
    let (renewed, uptime) = listen(&transport, wake, healthy, renew).await;
    healthy.send_replace(false);
    lock(reconnect).ended(renewed, uptime).unwrap_or_default()
}

async fn run(
    client: &reqwest::Client,
    session: &Session,
    wake: Arc<Notify>,
    healthy: watch::Sender<bool>,
    max_wait: &AtomicU64,
    reconnect: &Mutex<Reconnect>,
) {
    loop {
        healthy.send_replace(false);
        let wait = lock(reconnect).wait();
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        let next = grant(client, session).await;
        if !session.binding.is_current() {
            return;
        }
        let delay = match next {
            Ok((grant, expires)) => {
                max_wait.store(grant.max_poll_wait_ms, Ordering::Relaxed);
                let hub = &session.binding.hub;
                serve(hub, grant.transport, expires, &wake, &healthy, reconnect).await
            }
            // A rejected token is the HTTP owner's to resolve; until it does, back off like
            // any blocked grant rather than leaving a dead provider in place.
            Err(PollFailure::SignedOut | PollFailure::Unauthorized | PollFailure::Unsupported) => {
                lock(reconnect).failed(BLOCKED)
            }
            Err(PollFailure::Retry) => lock(reconnect).failed(RECONNECT),
        };
        lock(reconnect).not_before = Some(Instant::now() + delay);
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::realtime::REALTIME_PROTOCOL_VERSION;

    const API: &str = "https://api.flow-like.test";

    fn within(delay: Duration, nominal: u64) -> bool {
        let nominal = Duration::from_secs(nominal);
        delay >= nominal.mul_f64(0.8) && delay <= nominal.mul_f64(1.2)
    }

    fn nats(url: &str) -> RealtimeTransport {
        RealtimeTransport::Nats {
            url: url.into(),
            subject: "realtime.account".into(),
            token: "jwt".into(),
        }
    }

    fn azure(url: &str) -> RealtimeTransport {
        RealtimeTransport::AzureWebPubSub {
            url: url.into(),
            group: "account".into(),
        }
    }

    fn firebase(url: &str) -> RealtimeTransport {
        RealtimeTransport::GcpFirebaseRtdb {
            database_url: url.into(),
            api_key: "key".into(),
            project_id: "project".into(),
            custom_token: "token".into(),
            path: "realtime/account".into(),
        }
    }

    fn nats_url(transport: Option<RealtimeTransport>) -> Option<String> {
        match transport? {
            RealtimeTransport::Nats { url, .. } => Some(url),
            _ => None,
        }
    }

    #[test]
    fn an_https_api_refuses_cleartext_provider_grants_except_on_loopback() {
        for refused in [
            azure("ws://pubsub.example.test/client/hubs/realtime?access_token=secret"),
            firebase("http://project.firebaseio.test"),
            nats("ws://nats.example.test:8080"),
            nats("http://nats.example.test"),
            azure("not a url"),
        ] {
            assert!(secure_transport(API, refused).is_none());
        }
        for accepted in [
            azure("wss://pubsub.example.test/client/hubs/realtime"),
            azure("ws://localhost:7071/client"),
            azure("ws://127.0.0.1:7071/client"),
            azure("ws://[::1]:7071/client"),
            firebase("https://project.firebaseio.com"),
            firebase("http://127.0.0.1:9000"),
            RealtimeTransport::Http {},
        ] {
            assert_eq!(secure_transport(API, accepted.clone()), Some(accepted));
        }
        assert_eq!(
            nats_url(secure_transport(API, nats("nats://nats.example.test:4222"))).as_deref(),
            Some("tls://nats.example.test:4222")
        );
        for unchanged in [
            "tls://nats.example.test:4222",
            "wss://nats.example.test",
            "nats://localhost:4222",
            "nats://127.0.0.1:4222",
            "ws://localhost:8080",
        ] {
            assert_eq!(
                nats_url(secure_transport(API, nats(unchanged))).as_deref(),
                Some(unchanged)
            );
        }
        for transport in [
            azure("ws://pubsub.example.test/client"),
            firebase("http://project.firebaseio.test"),
            nats("nats://nats.example.test:4222"),
        ] {
            assert_eq!(
                secure_transport("http://localhost:8080", transport.clone()),
                Some(transport)
            );
        }
    }

    #[test]
    fn grants_renew_from_the_servers_remaining_lifetime() {
        let grant = |expires_at, expires_in| RealtimeConnectResponse {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            expires_at,
            expires_in,
            max_poll_wait_ms: 0,
            transport: RealtimeTransport::Http {},
        };
        let now = 1_000_000;
        assert_eq!(
            lifetime(&grant(now - 3_600, Some(900)), now),
            Some(Duration::from_secs(900))
        );
        assert_eq!(
            lifetime(&grant(now + 300, None), now),
            Some(Duration::from_secs(300))
        );
        for invalid in [
            grant(now + 900, Some(0)),
            grant(now + 900, Some(86_401)),
            grant(now - 1, None),
        ] {
            assert_eq!(lifetime(&invalid, now), None);
        }
    }

    #[test]
    fn reconnects_double_to_five_minutes_and_reset_only_after_a_stable_connection() {
        let mut reconnect = Reconnect::default();
        let delays: Vec<_> = (0..9)
            .map(|_| {
                reconnect
                    .ended(false, Some(Duration::from_secs(59)))
                    .unwrap()
            })
            .collect();
        for (delay, nominal) in delays.iter().zip([2, 4, 8, 16, 32, 64, 128, 256, 300]) {
            assert!(within(*delay, nominal), "{delay:?} for {nominal}");
        }
        let renewed = reconnect.ended(true, Some(Duration::from_secs(30)));
        assert!(renewed.is_none());
        assert!(within(reconnect.ended(false, None).unwrap(), 300));
        assert!(within(reconnect.ended(true, None).unwrap(), 300));
        assert!(within(
            reconnect
                .ended(false, Some(Duration::from_secs(60)))
                .unwrap(),
            2
        ));
        assert!(
            reconnect
                .ended(true, Some(Duration::from_secs(600)))
                .is_none()
        );
        assert!(within(reconnect.failed(RECONNECT), 2));
        assert!(within(reconnect.failed(BLOCKED), 60));
    }

    #[test]
    fn a_restarted_provider_waits_out_the_previous_backoff() {
        let pending = Reconnect {
            failures: 3,
            not_before: Some(Instant::now() + Duration::from_secs(30)),
        };
        assert!(pending.wait() > Duration::from_secs(29));
        assert_eq!(Reconnect::default().wait(), Duration::ZERO);
        let elapsed = Reconnect {
            failures: 3,
            not_before: Some(Instant::now() - Duration::from_secs(1)),
        };
        assert_eq!(elapsed.wait(), Duration::ZERO);
    }
}
