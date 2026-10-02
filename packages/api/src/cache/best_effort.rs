//! Bounded access to shared, disposable metadata. Callers retain authorization
//! and freshness checks; a failed invalidation never proves an entry is fresh.

use std::{
    future::Future,
    io,
    time::{Duration, Instant},
};

use serde::{Serialize, de::DeserializeOwned};

use super::{CacheBackendHandle, CacheStoreError};

const TIMEOUT: Duration = Duration::from_millis(250);
pub const MAX_ENTRY_BYTES: usize = 256 * 1024;

async fn operation<T>(
    namespace: &str,
    action: &str,
    future: impl Future<Output = Result<T, CacheStoreError>>,
) -> Option<T> {
    let started = Instant::now();
    match flow_like_types::tokio::time::timeout(TIMEOUT, future).await {
        Ok(Ok(value)) => {
            tracing::debug!(
                namespace,
                action,
                elapsed_ms = started.elapsed().as_millis() as u64,
                outcome = "ok",
                "Shared metadata cache"
            );
            Some(value)
        }
        Ok(Err(error)) => {
            tracing::debug!(namespace, action, error = %error, outcome = "error", "Shared metadata cache");
            None
        }
        Err(_) => {
            tracing::debug!(
                namespace,
                action,
                outcome = "timeout",
                "Shared metadata cache"
            );
            None
        }
    }
}

pub async fn get<T: DeserializeOwned>(
    handle: &CacheBackendHandle,
    namespace: &str,
    key: &str,
) -> Option<T> {
    let value = operation(namespace, "get", async {
        handle.platform().await?.get(namespace, key).await
    })
    .await
    .flatten();
    tracing::debug!(
        namespace,
        hit = value.is_some(),
        "Shared metadata cache lookup"
    );
    value
}

// Count serialized bytes without allocating an oversized buffer. The platform
// cache bypasses app-route limits, so every metadata write needs its own bound.
fn fits<T: Serialize>(value: &T) -> bool {
    struct BoundedSize(usize);
    impl io::Write for BoundedSize {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_ENTRY_BYTES.saturating_sub(self.0) {
                return Err(io::Error::other("metadata exceeds cache size limit"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(BoundedSize(0), value).is_ok()
}

pub async fn set<T: Serialize>(
    handle: &CacheBackendHandle,
    namespace: &str,
    key: &str,
    value: &T,
    ttl: Duration,
) {
    if ttl.is_zero() || !fits(value) {
        tracing::debug!(
            namespace,
            outcome = "skipped",
            "Shared metadata cache value exceeds limit or has no lifetime"
        );
        return;
    }
    operation(namespace, "set", async {
        handle
            .platform()
            .await?
            .set(namespace, key, value, ttl)
            .await
    })
    .await;
}

pub async fn delete(handle: &CacheBackendHandle, namespace: &str, key: &str) {
    operation(namespace, "delete", async {
        handle.platform().await?.delete(namespace, key).await
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_encoded_json_including_escaping() {
        assert!(fits(&"x".repeat(MAX_ENTRY_BYTES - 2)));
        assert!(!fits(&"x".repeat(MAX_ENTRY_BYTES - 1)));
        assert!(!fits(&"\n".repeat(MAX_ENTRY_BYTES / 2)));
        assert!(fits(&serde_json::json!({"small": [1, 2, 3]})));
    }

    #[flow_like_types::tokio::test]
    async fn backend_errors_and_timeouts_fall_back() {
        assert!(
            operation::<()>("test", "read", async {
                Err(CacheStoreError::Connection("unavailable".into()))
            })
            .await
            .is_none()
        );
        assert!(
            operation::<()>("test", "read", std::future::pending())
                .await
                .is_none()
        );
        assert_eq!(operation("test", "read", async { Ok(7) }).await, Some(7));
    }
}
