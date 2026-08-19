use axum::{
    Router,
    routing::{delete, post},
};

use crate::state::AppState;

pub mod batch;
pub mod delete_files;
pub mod download_files;
pub mod list_files;
pub mod paths;
pub mod presign_data_access;
pub mod upload_files;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            delete(delete_files::delete_files).put(upload_files::upload_files),
        )
        .route(
            "/user",
            delete(delete_files::delete_user_files).put(upload_files::upload_user_files),
        )
        .route("/presign", post(presign_data_access::presign_data_access))
        .route(
            "/user/presign",
            post(presign_data_access::presign_user_data_access),
        )
        .route("/download", post(download_files::download_files))
        .route("/user/download", post(download_files::download_user_files))
        .route("/list", post(list_files::list_files))
        .route("/user/list", post(list_files::list_user_files))
}
