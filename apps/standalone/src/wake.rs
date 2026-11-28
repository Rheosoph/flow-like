//! A push doorbell for the device's polling loops.
//!
//! The realtime channel carries no data, commands or credentials: a valid wakeup only makes
//! the device run its existing, device-authenticated API calls sooner. A forged or replayed
//! wakeup can therefore at most cause extra polls, which `MIN_GAP` bounds.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};
use tokio::sync::{Notify, watch};

pub const MIN_GAP: Duration = Duration::from_secs(2);
pub const WATCHED_WINDOW: Duration = Duration::from_secs(300);

pub struct DeviceWake {
    generation: watch::Sender<u64>,
    live: watch::Sender<bool>,
    last: Mutex<Option<Instant>>,
    doorbell: Arc<Notify>,
}

impl Default for DeviceWake {
    fn default() -> Self {
        Self {
            generation: watch::channel(0).0,
            live: watch::channel(false).0,
            last: Mutex::new(None),
            doorbell: Arc::new(Notify::new()),
        }
    }
}

impl DeviceWake {
    pub fn subscribe(self: &Arc<Self>) -> WakeSubscription {
        WakeSubscription {
            generation: self.generation.subscribe(),
            wake: Arc::clone(self),
        }
    }

    /// Bumps the wake generation unless one was admitted less than `MIN_GAP` before `now`.
    pub fn admit_at(&self, now: Instant) -> bool {
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| now.saturating_duration_since(at) < MIN_GAP) {
            return false;
        }
        *last = Some(now);
        self.generation.send_modify(|generation| *generation += 1);
        true
    }

    #[cfg(test)]
    pub(crate) fn set_live(&self, live: bool) {
        self.live.send_replace(live);
    }

    #[cfg(feature = "runtime")]
    fn live_sender(&self) -> watch::Sender<bool> {
        self.live.clone()
    }

    #[cfg(feature = "runtime")]
    fn doorbell(&self) -> Arc<Notify> {
        Arc::clone(&self.doorbell)
    }

    #[cfg(feature = "runtime")]
    async fn pump(&self) {
        loop {
            self.doorbell.notified().await;
            self.admit_at(Instant::now());
            tokio::time::sleep(MIN_GAP).await;
        }
    }
}

#[derive(Clone)]
pub struct WakeSubscription {
    generation: watch::Receiver<u64>,
    wake: Arc<DeviceWake>,
}

impl WakeSubscription {
    /// Resolves when a wakeup was admitted since the last call.
    pub async fn changed(&mut self) {
        if self.generation.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    pub fn is_live(&self) -> bool {
        *self.wake.live.borrow()
    }

    pub fn watched_at(&self, now: Instant) -> bool {
        self.wake
            .last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some_and(|at| now.saturating_duration_since(at) < WATCHED_WINDOW)
    }

    pub fn watched(&self) -> bool {
        self.watched_at(Instant::now())
    }
}

#[cfg(feature = "runtime")]
pub use grant::run;

#[cfg(feature = "runtime")]
mod grant {
    use super::DeviceWake;
    use crate::enrollment::DeviceSession;
    use flow_like_types::realtime::{
        REALTIME_PROTOCOL_VERSION, RealtimeConnectRequest, RealtimeConnectResponse,
        RealtimeTransport,
    };
    use rand_core::RngCore;
    use reqwest::Method;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    use tokio_util::sync::CancellationToken;

    const MAX_GRANT_BYTES: usize = 64 * 1024;
    const RENEWAL_MARGIN: Duration = Duration::from_secs(20);
    const STABLE: Duration = Duration::from_secs(60);
    const BACKOFF_BASE: Duration = Duration::from_secs(1);
    const BACKOFF_CAP: Duration = Duration::from_secs(300);
    const HTTP_RETRY_MIN: Duration = Duration::from_secs(30);
    const HTTP_RETRY_MAX: Duration = Duration::from_secs(900);
    const UNSUPPORTED_RETRY: Duration = Duration::from_secs(900);

    #[derive(Debug, PartialEq)]
    pub(super) enum Plan {
        Listen(RealtimeTransport, Duration),
        Retry(Duration),
        Failed,
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
        reqwest::Url::parse(url).is_ok_and(|url| {
            url.scheme() == secure || (url.scheme() == cleartext && loopback(&url))
        })
    }

    /// An HTTPS API never hands wakeup credentials to a cleartext provider unless it is on
    /// loopback. NATS without a TLS scheme is upgraded so the client requires TLS.
    pub(super) fn secure_transport(
        hub: &str,
        transport: RealtimeTransport,
    ) -> Option<RealtimeTransport> {
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

    fn lifetime(grant: &RealtimeConnectResponse, now: i64) -> Option<Duration> {
        let seconds = match grant.expires_in {
            Some(seconds) => seconds,
            None => u64::try_from(grant.expires_at.saturating_sub(now)).ok()?,
        };
        (1..=86_400)
            .contains(&seconds)
            .then(|| Duration::from_secs(seconds))
    }

    pub(super) fn plan(hub: &str, status: u16, body: &[u8], now: i64) -> Plan {
        if matches!(status, 404 | 405 | 501) {
            return Plan::Retry(UNSUPPORTED_RETRY);
        }
        if status != 200 {
            return Plan::Failed;
        }
        let Ok(grant) = serde_json::from_slice::<RealtimeConnectResponse>(body) else {
            return Plan::Failed;
        };
        if grant.protocol_version != REALTIME_PROTOCOL_VERSION {
            return Plan::Failed;
        }
        let Some(remaining) = lifetime(&grant, now) else {
            return Plan::Failed;
        };
        let Some(transport) = secure_transport(hub, grant.transport) else {
            tracing::warn!(
                "Refused a cleartext realtime provider grant from an HTTPS API; polling over HTTP"
            );
            return Plan::Failed;
        };
        if matches!(transport, RealtimeTransport::Http {}) {
            return Plan::Retry(remaining.clamp(HTTP_RETRY_MIN, HTTP_RETRY_MAX));
        }
        let renew = remaining
            .saturating_sub(RENEWAL_MARGIN)
            .max(Duration::from_secs(1));
        Plan::Listen(transport, renew)
    }

    #[derive(Default)]
    pub(super) struct Backoff {
        failures: u32,
    }

    impl Backoff {
        pub(super) fn failed(&mut self) -> Duration {
            self.failures = self.failures.saturating_add(1);
            BACKOFF_BASE
                .saturating_mul(2u32.saturating_pow(self.failures - 1))
                .min(BACKOFF_CAP)
        }

        pub(super) fn ended(&mut self, renewed: bool, uptime: Option<Duration>) -> Duration {
            if uptime.is_some_and(|uptime| uptime >= STABLE) {
                self.failures = 0;
            }
            if renewed && uptime.is_some() {
                Duration::ZERO
            } else {
                self.failed()
            }
        }
    }

    async fn listen(
        transport: &RealtimeTransport,
        wake: &DeviceWake,
        renew: Duration,
    ) -> (bool, Option<Duration>) {
        let healthy = wake.live_sender();
        let mut up = healthy.subscribe();
        let listener =
            flow_like_channels::realtime::listen(transport, wake.doorbell(), healthy, None);
        tokio::pin!(listener);
        let deadline = tokio::time::sleep(renew);
        tokio::pin!(deadline);
        let mut connected = None;
        let renewed = loop {
            tokio::select! {
                _ = &mut listener => break false,
                () = &mut deadline => break true,
                Ok(()) = up.changed(), if connected.is_none() => {
                    if *up.borrow_and_update() {
                        connected = Some(Instant::now());
                    }
                }
            }
        };
        (renewed, connected.map(|at: Instant| at.elapsed()))
    }

    async fn next(device: &DeviceSession, backoff: &mut Backoff, wake: &DeviceWake) -> Duration {
        let body = serde_json::to_value(RealtimeConnectRequest::default()).ok();
        let path = format!("/devices/{}/realtime/connect", device.device_id());
        let answer = device
            .api_call(Method::POST, &path, body.as_ref(), MAX_GRANT_BYTES)
            .await;
        let now = chrono::Utc::now().timestamp();
        let plan = match answer {
            Ok((status, body)) => plan(device.api_base_url(), status, &body, now),
            Err(error) => {
                tracing::debug!("Realtime grant failed: {error:#}");
                Plan::Failed
            }
        };
        match plan {
            Plan::Retry(delay) => delay,
            Plan::Failed => backoff.failed(),
            Plan::Listen(transport, renew) => {
                let (renewed, uptime) = listen(&transport, wake, renew).await;
                wake.live_sender().send_replace(false);
                backoff.ended(renewed, uptime)
            }
        }
    }

    /// Keeps the doorbell connected until `cancel`; the wake is not live while it is down.
    pub async fn run(
        device: Arc<DeviceSession>,
        wake: Arc<DeviceWake>,
        cancel: CancellationToken,
    ) -> anyhow::Result<()> {
        let mut backoff = Backoff::default();
        let work = async {
            loop {
                wake.live_sender().send_replace(false);
                let delay = next(&device, &mut backoff, &wake).await;
                let jitter = u64::from(rand_core::OsRng.next_u32()) % 250;
                tokio::time::sleep(delay + Duration::from_millis(jitter)).await;
            }
        };
        tokio::select! {
            () = cancel.cancelled() => {}
            () = work => {}
            () = wake.pump() => {}
        }
        wake.live_sender().send_replace(false);
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;

        const API: &str = "https://api.flow-like.test";

        fn grant(transport: serde_json::Value, expires_in: u64) -> Vec<u8> {
            serde_json::to_vec(&json!({
                "protocol_version": REALTIME_PROTOCOL_VERSION,
                "expires_at": 0,
                "expires_in": expires_in,
                "transport": transport,
            }))
            .unwrap()
        }

        #[test]
        fn an_http_grant_is_not_live_and_retries_within_bounds() {
            let http = json!({"type": "http"});
            for (seconds, expected) in [(5, 30), (120, 120), (86_000, 900)] {
                assert_eq!(
                    plan(API, 200, &grant(http.clone(), seconds), 0),
                    Plan::Retry(Duration::from_secs(expected))
                );
            }
        }

        #[test]
        fn an_older_api_without_the_route_is_retried_rarely() {
            for status in [404, 405, 501] {
                assert_eq!(
                    plan(API, status, b"", 0),
                    Plan::Retry(Duration::from_secs(900))
                );
            }
            assert_eq!(plan(API, 500, b"", 0), Plan::Failed);
            assert_eq!(plan(API, 200, b"nonsense", 0), Plan::Failed);
        }

        #[test]
        fn a_cleartext_provider_from_an_https_api_is_refused() {
            let cleartext = json!({"type": "azure_web_pubsub", "url": "ws://pubsub.example.test/c", "group": "g"});
            assert_eq!(
                plan(API, 200, &grant(cleartext.clone(), 600), 0),
                Plan::Failed
            );
            assert!(matches!(
                plan("http://localhost:8080", 200, &grant(cleartext, 600), 0),
                Plan::Listen(..)
            ));
            let nats = |url: &str| RealtimeTransport::Nats {
                url: url.into(),
                subject: "s".into(),
                token: "t".into(),
            };
            assert_eq!(
                secure_transport(API, nats("nats://nats.example.test:4222")),
                Some(nats("tls://nats.example.test:4222"))
            );
            assert!(secure_transport(API, nats("ws://nats.example.test")).is_none());
        }

        #[test]
        fn a_push_grant_renews_before_it_expires() {
            let secure = json!({"type": "azure_web_pubsub", "url": "wss://pubsub.example.test/c", "group": "g"});
            let Plan::Listen(_, renew) = plan(API, 200, &grant(secure, 600), 0) else {
                panic!("a secure grant is listened to");
            };
            assert_eq!(renew, Duration::from_secs(580));
        }

        #[test]
        fn failures_back_off_from_one_second_to_five_minutes_until_stable() {
            let mut backoff = Backoff::default();
            let delays: Vec<u64> = (0..10).map(|_| backoff.failed().as_secs()).collect();
            assert_eq!(delays, [1, 2, 4, 8, 16, 32, 64, 128, 256, 300]);
            assert_eq!(
                backoff.ended(true, Some(Duration::from_secs(10))),
                Duration::ZERO
            );
            assert_eq!(
                backoff
                    .ended(false, Some(Duration::from_secs(61)))
                    .as_secs(),
                1
            );
            assert_eq!(backoff.ended(false, None).as_secs(), 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rapid_wakeups_are_admitted_once_and_every_subscriber_sees_it() {
        let wake = Arc::new(DeviceWake::default());
        let mut first = wake.subscribe();
        let mut second = wake.subscribe();
        let start = Instant::now();
        assert!(wake.admit_at(start));
        assert!(!wake.admit_at(start + Duration::from_millis(500)));
        assert!(!wake.admit_at(start + Duration::from_millis(1900)));
        for subscription in [&mut first, &mut second] {
            tokio::time::timeout(Duration::from_secs(1), subscription.changed())
                .await
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(20), subscription.changed())
                    .await
                    .is_err()
            );
        }
        assert!(wake.admit_at(start + MIN_GAP));
        tokio::time::timeout(Duration::from_secs(1), first.changed())
            .await
            .unwrap();
    }

    #[test]
    fn the_live_flag_follows_the_listener_and_watching_lapses() {
        let wake = Arc::new(DeviceWake::default());
        let subscription = wake.subscribe();
        let start = Instant::now();
        assert!(!subscription.is_live());
        assert!(!subscription.watched_at(start));
        wake.live.send_replace(true);
        assert!(subscription.is_live());
        wake.live.send_replace(false);
        assert!(!subscription.is_live());
        wake.admit_at(start);
        assert!(subscription.watched_at(start + Duration::from_secs(299)));
        assert!(!subscription.watched_at(start + WATCHED_WINDOW));
    }
}
