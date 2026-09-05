//! Request audit context shared by the mutation middleware and domain hooks.
//! The context lasts until the response headers are ready. Background work must
//! record its own lifecycle events because task-local values are not inherited.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use flow_like_types::tokio;

#[derive(Clone, Default)]
pub(crate) struct RequestAuditContext {
    pub actor_ip: Option<String>,
    pub failures: Arc<AtomicUsize>,
}

tokio::task_local! {
    pub(crate) static REQUEST_AUDIT: RequestAuditContext;
}

pub fn actor_ip() -> Option<String> {
    REQUEST_AUDIT
        .try_with(|context| context.actor_ip.clone())
        .ok()
        .flatten()
}

pub fn record_failure() {
    let _ = REQUEST_AUDIT.try_with(|context| {
        context.failures.fetch_add(1, Ordering::Relaxed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_context_is_isolated_and_counts_failures() {
        assert_eq!(actor_ip(), None);
        let context = RequestAuditContext {
            actor_ip: Some("192.0.2.1".to_string()),
            ..Default::default()
        };
        let failures = context.failures.clone();
        REQUEST_AUDIT
            .scope(context, async {
                assert_eq!(actor_ip().as_deref(), Some("192.0.2.1"));
                record_failure();
                record_failure();
                tokio::spawn(async {
                    assert_eq!(actor_ip(), None);
                    record_failure();
                })
                .await
                .unwrap();
            })
            .await;
        assert_eq!(failures.load(Ordering::Relaxed), 2);
        assert_eq!(actor_ip(), None);
    }
}
