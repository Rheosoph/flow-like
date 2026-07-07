pub mod copilot;
pub mod global_chat;
pub mod governance;

use axum::Router;

use crate::State;

pub fn routes() -> Router<std::sync::Arc<State>> {
    Router::new()
        .nest("/copilot", copilot::routes())
        .nest("/global-chat", global_chat::routes())
}
