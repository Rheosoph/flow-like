use crate::{
    certificates,
    enrollment::{DeviceSession, unix_time},
    state::StateStore,
};
use anyhow::Result;
use flow_like_device_protocol::CertificateInventory;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const POLL: Duration = Duration::from_secs(5);
const REFRESH: Duration = Duration::from_secs(3600);

#[derive(Default)]
struct Publication {
    revision: Option<u64>,
    published_at: Option<Instant>,
    retry_at: Option<Instant>,
    failures: u32,
}

impl Publication {
    fn due(&self, revision: u64, now: Instant) -> bool {
        self.retry_at.is_none_or(|at| now >= at)
            && (self.revision != Some(revision)
                || self
                    .published_at
                    .is_none_or(|at| now.saturating_duration_since(at) >= REFRESH))
    }

    fn success(&mut self, revision: u64, now: Instant) {
        self.revision = Some(revision);
        self.published_at = Some(now);
        self.retry_at = None;
        self.failures = 0;
    }

    fn failure(&mut self, now: Instant) {
        let delay = Duration::from_secs((5_u64 << self.failures.min(4)).min(60));
        self.failures = self.failures.saturating_add(1);
        self.retry_at = Some(now + delay);
    }
}

fn pending(root: &Path, publication: &Publication) -> Result<Option<CertificateInventory>> {
    let store = StateStore::open(&root.join("management.sqlite"))?;
    if !publication.due(certificates::inventory_revision(&store)?, Instant::now()) {
        return Ok(None);
    }
    Ok(Some(certificates::inventory(&store, unix_time()?)?))
}

/// The local SQLite inventory is the retry source, including deletions. Restarting
/// republishes it; the API deduplicates the same material revision.
pub async fn publish(
    root: PathBuf,
    device: Arc<DeviceSession>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut publication = Publication::default();
    let mut tick = tokio::time::interval(POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! { _ = cancel.cancelled() => return Ok(()), _ = tick.tick() => {} }
        if publication.retry_at.is_some_and(|at| Instant::now() < at) {
            continue;
        }
        let result = async {
            let Some(inventory) = pending(&root, &publication)? else {
                return Ok(None);
            };
            device.publish_certificate_inventory(&inventory).await?;
            Ok::<_, anyhow::Error>(Some(inventory.revision))
        };
        let result =
            tokio::select! { _ = cancel.cancelled() => return Ok(()), result = result => result };
        match result {
            Ok(Some(revision)) => publication.success(revision, Instant::now()),
            Ok(None) => {}
            Err(error) => {
                publication.failure(Instant::now());
                tracing::warn!(%error, "Certificate expiration inventory publication will retry");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
    };
    use flow_like_device_protocol::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn failures_preserve_acknowledged_revision_and_coalesce_changes_with_bounded_backoff() {
        let now = Instant::now();
        let mut publication = Publication::default();
        assert!(publication.due(1, now));
        publication.success(1, now);
        assert!(!publication.due(1, now + Duration::from_secs(30)));
        assert!(publication.due(2, now + Duration::from_secs(30)));
        publication.failure(now);
        assert!(!publication.due(3, now + Duration::from_secs(4)));
        assert!(publication.due(3, now + Duration::from_secs(5)));
        for _ in 0..100 {
            publication.failure(now);
        }
        assert_eq!(publication.retry_at, Some(now + Duration::from_secs(60)));
        assert_eq!(publication.revision, Some(1));
        publication.success(3, now);
        assert!(publication.due(3, now + REFRESH));
        assert!(!publication.due(3, now + REFRESH - Duration::from_secs(1)));
        assert_eq!(publication.failures, 0);
    }

    #[derive(Clone)]
    struct Cloud {
        key: Ed25519PublicKey,
        base: String,
        token_calls: Arc<AtomicUsize>,
        uploads: Arc<Mutex<Vec<CertificateInventory>>>,
        proofs: Arc<Mutex<std::collections::HashSet<String>>>,
    }

    async fn token(
        State(cloud): State<Cloud>,
        Json(request): Json<DeviceTokenRequest>,
    ) -> Json<DeviceTokenResponse> {
        let now = unix_time().unwrap();
        verify_client_assertion(
            &request.client_assertion,
            &cloud.key,
            "device",
            &format!("{}/devices/token", cloud.base),
            now,
        )
        .unwrap();
        cloud.token_calls.fetch_add(1, Ordering::SeqCst);
        Json(DeviceTokenResponse {
            access_token: "certificate-session".into(),
            token_type: "DPoP".into(),
            expires_in: 600,
            expires_at: now + 600,
        })
    }

    async fn upload(
        State(cloud): State<Cloud>,
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        assert_eq!(body.as_object().unwrap().len(), 1);
        assert_eq!(headers["authorization"], "DPoP certificate-session");
        let proof = verify_dpop(
            headers["dpop"].to_str().unwrap(),
            &cloud.key,
            &DpopContext {
                method: "PUT",
                url: &format!("{}/devices/device/certificate-inventory", cloud.base),
                access_token: Some("certificate-session"),
                nonce: None,
                key_thumbprint: &cloud.key.thumbprint().unwrap(),
                now: unix_time().unwrap(),
            },
        )
        .unwrap();
        assert!(cloud.proofs.lock().unwrap().insert(proof.jti));
        let inventory = verify_certificate_inventory(
            body["inventory_jws"].as_str().unwrap(),
            &cloud.key,
            "device",
            unix_time().unwrap(),
        )
        .unwrap();
        let mut uploads = cloud.uploads.lock().unwrap();
        uploads.push(inventory.clone());
        if uploads.len() == 1 {
            return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({})));
        }
        (
            StatusCode::OK,
            Json(
                serde_json::json!({"revision":inventory.revision,"updated_at":unix_time().unwrap(),"certificates":inventory.certificates}),
            ),
        )
    }

    #[tokio::test]
    async fn retries_sign_fresh_minimal_inventory_and_reuse_bound_session_with_deletions()
    -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let key = SigningKey::generate();
        let cloud = Cloud {
            key: key.public_key(),
            base: format!("http://{}/api/v1", listener.local_addr()?),
            token_calls: Default::default(),
            uploads: Default::default(),
            proofs: Default::default(),
        };
        let router = Router::new()
            .route("/api/v1/devices/token", post(token))
            .route(
                "/api/v1/devices/device/certificate-inventory",
                axum::routing::put(upload),
            )
            .with_state(cloud.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let device = DeviceSession::test_session(cloud.base.clone(), "device".into(), key);
        let mut inventory = CertificateInventory {
            version: 1,
            device_id: "device".into(),
            revision: 4,
            issued_at: 1,
            certificates: vec![CertificateInventoryEntry {
                certificate_id: "a48aeb81-1321-402e-b09c-8b304f9c3c2f".into(),
                revision: 2,
                fingerprint_sha256: "ab".repeat(32),
                not_after: unix_time()? + 86400,
            }],
        };
        assert!(
            device
                .publish_certificate_inventory(&inventory)
                .await
                .is_err()
        );
        device.publish_certificate_inventory(&inventory).await?;
        inventory.revision += 1;
        inventory.certificates.clear();
        device.publish_certificate_inventory(&inventory).await?;
        assert_eq!(cloud.token_calls.load(Ordering::SeqCst), 1);
        let uploads = cloud.uploads.lock().unwrap();
        assert_eq!(uploads.len(), 3);
        assert!(uploads.iter().all(|inventory| inventory.issued_at > 1));
        assert_eq!(uploads[0].revision, uploads[1].revision);
        assert_eq!(uploads[0].certificates, uploads[1].certificates);
        assert!(uploads[2].certificates.is_empty());
        assert_eq!(cloud.proofs.lock().unwrap().len(), 3);
        server.abort();
        Ok(())
    }
}
