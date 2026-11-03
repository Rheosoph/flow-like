use super::{ready, valid_wakeup};
use flow_like_types::{Result, anyhow, bail};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::sync::{Notify, watch};

pub(super) async fn listen(
    url: &str,
    subject: &str,
    token: &str,
    wake: &Notify,
    healthy: &watch::Sender<bool>,
) -> Result<()> {
    if subject.is_empty()
        || subject.len() > 256
        || subject
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '*' | '>'))
    {
        bail!("Realtime NATS subject must be exact");
    }
    crate::nats::validate_url(url)?;
    let token = token.to_owned();
    let (disconnected, mut lost) = watch::channel(false);
    // Observe connection events so even a fast reconnect cannot hide a gap.
    // The owner obtains a fresh grant and replays after this listener returns.
    let options = async_nats::ConnectOptions::with_auth_callback(move |_| {
        let token = token.clone();
        async move {
            let mut auth = async_nats::Auth::new();
            auth.jwt = Some(token);
            Ok(auth)
        }
    })
    .event_callback(move |event| {
        let disconnected = disconnected.clone();
        async move {
            if matches!(
                event,
                async_nats::Event::Disconnected
                    | async_nats::Event::Closed
                    | async_nats::Event::SlowConsumer(_)
                    | async_nats::Event::ServerError(_)
                    | async_nats::Event::ClientError(_)
            ) {
                disconnected.send_replace(true);
            }
        }
    })
    .name("flow-like-realtime")
    .tls_client_config(crate::tls::client_config()?.as_ref().clone())
    .ignore_discovered_servers()
    .ping_interval(Duration::from_secs(10))
    .max_reconnects(1)
    .connection_timeout(Duration::from_secs(10));
    let client = tokio::time::timeout(Duration::from_secs(10), options.connect(url))
        .await
        .map_err(|_| anyhow!("Realtime NATS connection timed out"))?
        .map_err(|_| anyhow!("Realtime NATS connection failed"))?;
    let mut subscription = client
        .subscribe(subject.to_owned())
        .await
        .map_err(|_| anyhow!("Realtime NATS subscription failed"))?;
    tokio::time::timeout(Duration::from_secs(10), client.flush())
        .await
        .map_err(|_| anyhow!("Realtime NATS subscription timed out"))?
        .map_err(|_| anyhow!("Realtime NATS subscription failed"))?;
    ready(healthy, wake);
    let mut check = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = lost.changed() => bail!("Realtime NATS connection interrupted"),
            message = subscription.next() => {
                let message = message.ok_or_else(|| anyhow!("Realtime NATS subscription closed"))?;
                if message.subject.as_str() == subject && valid_wakeup(&message.payload) {
                    wake.notify_one();
                }
            }
            _ = check.tick() => {
                if client.connection_state() != async_nats::connection::State::Connected {
                    bail!("Realtime NATS connection interrupted");
                }
            }
        }
    }
}
