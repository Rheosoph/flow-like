use crate::{error::ApiError, state::AppState};
use flow_like_storage::{Path as ObjectPath, object_store::ObjectStoreExt};
use flow_like_types::tokio;
use std::time::Duration;

const TEMPORARY_RETENTION: &[(&str, i64)] = &[
    ("tmp/user", 32 * 86_400),
    ("tmp/solution-staging", 8 * 86_400),
];

fn expired_temporary_object(
    path: &ObjectPath,
    modified: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    TEMPORARY_RETENTION.iter().any(|(prefix, max_age)| {
        path.prefix_matches(&ObjectPath::from(*prefix)) && (now - modified).num_seconds() > *max_age
    })
}

/// Runs from scheduled maintenance, including deployments without a persistent process.
pub(crate) async fn cleanup_expired(state: &AppState) -> Result<(), ApiError> {
    use futures::TryStreamExt;
    let store = state.master_credentials().await?.to_store(false).await?;
    let generic = store.as_generic();
    let now = chrono::Utc::now();
    for (prefix, _) in TEMPORARY_RETENTION {
        let prefix = ObjectPath::from(*prefix);
        let mut objects = generic.list(Some(&prefix));
        while let Some(object) = objects.try_next().await? {
            if expired_temporary_object(&object.location, object.last_modified, now) {
                generic.delete(&object.location).await?;
            }
        }
    }
    Ok(())
}

pub(crate) fn spawn_cleanup(state: &AppState) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let state = std::sync::Arc::downgrade(state);
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            let Some(state) = state.upgrade() else {
                break;
            };
            if let Err(error) = cleanup_expired(&state).await {
                tracing::warn!(%error, "Expired upload cleanup failed");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_only_expires_temporary_directories_after_their_retention() {
        let now = chrono::Utc::now();
        let old = now - chrono::Duration::days(33);
        for key in ["tmp/user/u/file", "tmp/solution-staging/u/file"] {
            assert!(expired_temporary_object(&ObjectPath::from(key), old, now));
            assert!(!expired_temporary_object(&ObjectPath::from(key), now, now));
        }
        for key in [
            "apps/a/upload/file",
            "solution-requests/files/request/file",
            "tmp/user-backups/file",
            "tmp/solution-staging-backups/file",
        ] {
            assert!(!expired_temporary_object(&ObjectPath::from(key), old, now));
        }
        assert!(!expired_temporary_object(
            &ObjectPath::from("tmp/user/u/file"),
            now - chrono::Duration::days(32),
            now,
        ));
        assert!(!expired_temporary_object(
            &ObjectPath::from("tmp/solution-staging/u/file"),
            now - chrono::Duration::days(7),
            now,
        ));
    }
}
