pub(crate) mod get_current_user_info;
pub(crate) mod get_executing_user;
pub(crate) mod has_attribute;
pub(crate) mod has_permission;
pub(crate) mod is_technical_user;
pub(crate) mod project_users;

pub use get_current_user_info::*;
pub use get_executing_user::*;
pub use has_attribute::*;
pub use has_permission::*;
pub use is_technical_user::*;
pub use project_users::*;

use flow_like::flow::execution::{ExecutionEnvironment, context::ExecutionContext};

/// The hub route that answers who a run acts as. Runs that hold an executor
/// token (server runs and their replays) ask the execution route of their app;
/// every other run asks with its user's own token.
pub(crate) fn run_user_route(context: &ExecutionContext) -> String {
    let cache = context.execution_cache.as_ref();
    user_route(
        context.executor_api_auth.is_some(),
        cache.is_some_and(|cache| cache.shadow),
        context.execution_environment(),
        cache.map_or("", |cache| cache.app_id.as_str()),
    )
}

fn user_route(
    executor_auth: bool,
    shadow: bool,
    environment: ExecutionEnvironment,
    app_id: &str,
) -> String {
    let executor_run = executor_auth || shadow || environment == ExecutionEnvironment::Server;
    let app_id = app_id.trim();
    if executor_run && !app_id.is_empty() {
        return format!("execution/apps/{}/user", urlencoding::encode(app_id));
    }
    "user/info".to_string()
}

#[cfg(test)]
mod tests {
    use super::user_route;
    use flow_like::flow::execution::ExecutionEnvironment::{Desktop, Local, Server};

    #[test]
    fn runs_with_an_executor_token_ask_the_execution_route() {
        for (executor_auth, shadow, environment, app_id, expected) in [
            (true, false, Server, "app-1", "execution/apps/app-1/user"),
            (true, false, Local, "app-1", "execution/apps/app-1/user"),
            (false, true, Local, "app-1", "execution/apps/app-1/user"),
            (false, true, Server, "app-1", "execution/apps/app-1/user"),
            (false, false, Server, "app-1", "execution/apps/app-1/user"),
            (
                true,
                false,
                Server,
                "app/1 x",
                "execution/apps/app%2F1%20x/user",
            ),
            (false, false, Local, "app-1", "user/info"),
            (false, false, Desktop, "app-1", "user/info"),
            (true, true, Server, "  ", "user/info"),
        ] {
            assert_eq!(
                user_route(executor_auth, shadow, environment, app_id),
                expected,
                "executor_auth = {executor_auth}, shadow = {shadow}, {environment:?}, {app_id:?}"
            );
        }
    }
}
