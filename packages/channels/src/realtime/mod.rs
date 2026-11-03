//! Read-only cloud wakeups. Account data and replay cursors always come from the API.
#![cfg_attr(
    not(any(
        feature = "aws",
        feature = "aws-receiver",
        feature = "azure",
        feature = "gcp",
        feature = "nats"
    )),
    allow(dead_code, unused_variables)
)]

use flow_like_types::{
    Result, anyhow,
    realtime::{MAX_REALTIME_WAKEUP_BYTES, RealtimeTransport, RealtimeWakeup},
};
use std::sync::Arc;
use tokio::sync::{Notify, watch};

#[cfg(any(feature = "aws", feature = "aws-receiver"))]
mod aws;
#[cfg(feature = "azure")]
mod azure;
#[cfg(feature = "gcp")]
mod gcp;
#[cfg(feature = "nats")]
mod nats;

const MAX_WAKE_BYTES: usize = MAX_REALTIME_WAKEUP_BYTES;

fn valid_wakeup(payload: &[u8]) -> bool {
    if payload.len() > MAX_WAKE_BYTES {
        return false;
    }
    serde_json::from_slice::<RealtimeWakeup>(payload).is_ok_and(|wake| wake.is_valid())
}

fn ready(healthy: &watch::Sender<bool>, wake: &Notify) {
    healthy.send_replace(true);
    // Reconcile after every connection, including reconnects that missed a wakeup.
    wake.notify_one();
}

/// Runs one read-only subscription until it disconnects. Dropping this future closes it.
/// The owner renews grants and falls back to HTTP while this future is unavailable.
pub async fn listen(
    transport: &RealtimeTransport,
    wake: Arc<Notify>,
    healthy: watch::Sender<bool>,
) -> Result<()> {
    match transport {
        #[cfg(any(feature = "aws", feature = "aws-receiver"))]
        RealtimeTransport::AwsMqtt {
            endpoint,
            region,
            client_id,
            topic,
            credentials,
        } => {
            aws::listen(
                endpoint,
                region,
                client_id,
                topic,
                credentials,
                &wake,
                &healthy,
            )
            .await
        }
        #[cfg(feature = "azure")]
        RealtimeTransport::AzureWebPubSub { url, group } => {
            azure::listen(url, group, &wake, &healthy).await
        }
        #[cfg(feature = "gcp")]
        RealtimeTransport::GcpFirebaseRtdb {
            database_url,
            api_key,
            custom_token,
            path,
            ..
        } => gcp::listen(database_url, api_key, custom_token, path, &wake, &healthy).await,
        #[cfg(feature = "nats")]
        RealtimeTransport::Nats {
            url,
            subject,
            token,
        } => nats::listen(url, subject, token, &wake, &healthy).await,
        #[allow(unreachable_patterns)]
        _ => Err(anyhow!("Realtime provider has no compiled native listener")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wakeups_cannot_carry_execution_data_or_unbounded_content() {
        assert!(valid_wakeup(
            br#"{"protocol_version":1,"kind":"invalidate","nonce":"n1"}"#
        ));
        for value in [
            serde_json::json!({"protocol_version":2,"kind":"invalidate","nonce":"n"}),
            serde_json::json!({"protocol_version":1,"kind":"event","nonce":"n"}),
            serde_json::json!({"protocol_version":1,"kind":"invalidate","nonce":""}),
            serde_json::json!({"protocol_version":1,"kind":"invalidate","nonce":"n","run_id":"foreign"}),
            serde_json::json!({"protocol_version":1,"kind":"invalidate","nonce":"x".repeat(MAX_WAKE_BYTES)}),
        ] {
            assert!(!valid_wakeup(&serde_json::to_vec(&value).unwrap()));
        }
    }
}
