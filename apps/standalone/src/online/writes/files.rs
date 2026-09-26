use super::WriteManager;
use crate::outbox::BufferedFiles;
use anyhow::{Context, Result};
use flow_like_device_protocol::MAX_OFFLINE_OPERATION_BYTES;
use flow_like_offline_writes::{
    FileBuffering, FileOverlayOptions, FileRoute, Observation, OfflineErrorClassifier,
};
use flow_like_storage::object_store::ObjectStore;
use std::sync::Arc;

fn offline_error() -> OfflineErrorClassifier {
    Arc::new(|error| super::super::cache::is_offline(error).then_some(Observation::ConnectFailed))
}

pub(in crate::online) fn wrap(
    inner: Arc<dyn ObjectStore>,
    manager: Arc<WriteManager>,
    selected: &[BufferedFiles],
) -> Result<Arc<dyn ObjectStore>> {
    let mut routes = Vec::new();
    for selected in selected {
        let location = manager
            .credentials
            .locations
            .get(&selected.purpose)
            .context("Missing buffered file authorization")?;
        routes.push(FileRoute {
            purpose: selected.purpose,
            root: location.prefix.clone(),
            prefix: format!("{}/", selected.prefix),
            scheme: url::Url::parse(&location.uri)?.scheme().into(),
        });
    }
    let store: Arc<dyn ObjectStore> = manager.engine.file_overlay(
        inner,
        FileOverlayOptions {
            routes,
            buffering: FileBuffering::Always,
            max_file_bytes: MAX_OFFLINE_OPERATION_BYTES,
            offline_error: offline_error(),
        },
    )?;
    Ok(store)
}

#[cfg(test)]
#[path = "file_tests.rs"]
mod tests;
