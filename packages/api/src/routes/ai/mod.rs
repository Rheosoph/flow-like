pub mod copilot;
pub mod governance;

use axum::Router;

use crate::State;

pub fn routes() -> Router<std::sync::Arc<State>> {
    Router::new().nest("/copilot", copilot::routes())
}
