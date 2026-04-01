mod commands;
mod store;

use std::path::PathBuf;
use store::BlobStore;
use tauri::{
    Manager, RunEvent, Runtime,
    plugin::{Builder, TauriPlugin},
};

pub use commands::*;
pub use store::BlobRef;

pub fn init<R: Runtime>(blob_dir: Option<PathBuf>) -> TauriPlugin<R> {
    Builder::new("flow-like-dexie-blob-offload")
        .invoke_handler(tauri::generate_handler![
            commands::blob_store,
            commands::blob_get,
            commands::blob_store_batch,
            commands::blob_get_batch,
            commands::blob_delete,
            commands::blob_configure,
            commands::blob_inc_refs,
            commands::blob_dec_refs,
        ])
        .setup(move |app, _api| {
            let dir = blob_dir.unwrap_or_else(|| {
                app.path()
                    .app_data_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join("blob_store")
            });
            let store = BlobStore::new(dir.clone());
            store.load_ref_counts_sync(&dir);
            app.manage(store);
            Ok(())
        })
        .on_event(|_app, event| {
            if let RunEvent::Exit = event {
                tracing::debug!("flow-like-dexie-blob-offload plugin shutting down");
            }
        })
        .build()
}
