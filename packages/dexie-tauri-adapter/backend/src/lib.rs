mod commands;
mod sql;
mod store;

use sql::SqlStore;
use std::path::PathBuf;
use store::BlobStore;
use tauri::{
    Manager, RunEvent, Runtime,
    plugin::{Builder, TauriPlugin},
};

pub use commands::*;
pub use store::BlobRef;

pub fn init<R: Runtime>(blob_dir: Option<PathBuf>, sql_dir: Option<PathBuf>) -> TauriPlugin<R> {
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
            commands::sql_open,
            commands::sql_exec,
            commands::sql_close,
        ])
        .setup(move |app, _api| {
            let app_data = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            let dir = blob_dir.unwrap_or_else(|| app_data.join("blob_store"));
            let store = BlobStore::new(dir.clone());
            store.load_ref_counts_sync(&dir);
            app.manage(store);

            let sql_dir = sql_dir.unwrap_or_else(|| app_data.join("idb_sqlite"));
            app.manage(SqlStore::new(sql_dir));
            Ok(())
        })
        .on_event(|_app, event| {
            if let RunEvent::Exit = event {
                tracing::debug!("flow-like-dexie-blob-offload plugin shutting down");
            }
        })
        .build()
}
