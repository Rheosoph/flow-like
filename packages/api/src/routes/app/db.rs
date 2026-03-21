use axum::{
    Router,
    routing::{delete, get, post, put},
};

use crate::state::AppState;

pub mod add_column;
pub mod alter_column;
pub mod build_index;
pub mod db_add;
pub mod db_count;
pub mod db_delete;
pub mod db_list;
pub mod db_query;
pub mod db_update;
pub mod drop_columns;
pub mod drop_index;
pub mod get_db_schema;
pub mod get_indices;
pub mod list_tables;
pub mod optimize;
pub mod presign_db_access;
pub mod table_view;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_tables::list_tables))
        .route("/presign", post(presign_db_access::presign_db_access))
        .route(
            "/{table}",
            put(db_add::add_to_table)
                .delete(db_delete::delete_from_table)
                .get(db_list::list_items),
        )
        .route("/{table}/update", put(db_update::update_table))
        .route("/{table}/optimize", post(optimize::optimize_table))
        .route(
            "/{table}/columns",
            post(add_column::add_column)
                .put(alter_column::alter_column)
                .delete(drop_columns::drop_columns),
        )
        .route("/{table}/index", post(build_index::build_index))
        .route(
            "/{table}/index/{index_name}",
            delete(drop_index::drop_index),
        )
        .route("/{table}/query", post(db_query::query_table))
        .route("/{table}/schema", get(get_db_schema::get_db_schema))
        .route("/{table}/count", get(db_count::db_count))
        .route("/{table}/indices", get(get_indices::get_db_indices))
        .route("/{table}/view", get(table_view::table_view))
}
