use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{delete, get, patch, post, put},
};
use bit::{delete_bit, push_meta, upsert_bit};
use models::{sync_models, upsert_model};

use crate::state::AppState;

pub mod ai_act;
pub mod bit;
pub mod cache;
pub mod connections;
pub mod forks;
pub mod governance;
pub mod logs;
pub mod models;
pub mod packages;
pub mod profiles;
pub mod publication;
pub mod runs;
pub mod sinks;
pub mod solutions;
pub mod telemetry;
pub mod usage;
pub mod users;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/connections/graph",
            get(connections::get_global_connection_graph),
        )
        .route(
            "/bit/{bit_id}",
            put(upsert_bit::upsert_bit).delete(delete_bit::delete_bit),
        )
        .route("/bit/{bit_id}/{language}", put(push_meta::push_meta))
        .route("/models/sync", post(sync_models::sync_models))
        .route("/models/{slug}", put(upsert_model::upsert_model))
        .route(
            "/profiles/media",
            get(profiles::get_signed_profile_img_url::get_signed_profile_img_url),
        )
        .route(
            "/profiles/{profile_id}",
            put(profiles::upsert_profile_template::upsert_profile_template)
                .delete(profiles::delete_profile_template::delete_profile_template),
        )
        .route("/solutions", get(solutions::list_solutions::list_solutions))
        .route(
            "/solutions/{solution_id}",
            get(solutions::get_solution::get_solution)
                .patch(solutions::update_solution::update_solution),
        )
        .route(
            "/solutions/{solution_id}/logs",
            post(solutions::add_log::add_solution_log),
        )
        // Publication review routes
        .route(
            "/publication/requests",
            get(publication::get_requests::get_requests),
        )
        .route(
            "/publication/requests/{request_id}",
            patch(publication::upsert_requests::upsert_request),
        )
        .route(
            "/publication/suites",
            get(publication::get_group_requests::get_group_requests),
        )
        .route(
            "/publication/apps/{app_id}/content",
            get(publication::get_app_content::get_app_content),
        )
        .route(
            "/publication/apps/{app_id}/board/{board_id}",
            get(publication::get_board::get_board),
        )
        .route(
            "/publication/apps/{app_id}/page/{page_id}",
            get(publication::get_page::get_page),
        )
        // Package management routes
        .route("/packages", get(packages::get_packages::get_packages))
        .route("/packages/stats", get(packages::get_stats::get_stats))
        .route(
            "/packages/ensure-wasm-artifacts",
            post(packages::ensure_wasm_artifacts::ensure_wasm_artifacts),
        )
        .route(
            "/packages/{package_id}",
            get(packages::get_package::get_package)
                .patch(packages::update_package::update_package)
                .delete(packages::delete_package::delete_package),
        )
        .route(
            "/packages/{package_id}/review",
            post(packages::review_package::review_package),
        )
        // Sink token management routes
        .route(
            "/sinks",
            get(sinks::list_tokens::list_tokens).post(sinks::register_sink::register_sink),
        )
        .route("/sinks/{jti}", delete(sinks::revoke_sink::revoke_sink))
        // User management routes
        .route("/users", get(users::list_users::list_users))
        .route("/users/{user_id}", patch(users::update_user::update_user))
        // Usage reporting and limits
        .route("/usage/overview", get(usage::overview))
        .route("/usage/invocations", get(usage::invocations))
        .route("/usage/reconcile", post(usage::reconcile))
        .route("/usage/alerts", get(usage::alerts))
        .route(
            "/usage/alerts/{alert_id}/ack",
            post(usage::acknowledge_alert),
        )
        .route("/usage/audit", get(usage::audit))
        .route(
            "/usage/apps/{app_id}/limits",
            get(usage::get_limits).put(usage::put_limits),
        )
        .route(
            "/usage/apps/{app_id}/technical-users/{technical_user_id}/limits",
            get(usage::get_technical_user_limits).put(usage::put_technical_user_limits),
        )
        // Governance scores
        .route(
            "/governance/scores",
            get(governance::list_scores::list_scores),
        )
        .route(
            "/governance/scores/summary",
            get(governance::get_scores_summary::get_scores_summary),
        )
        .route(
            "/governance/scores/recompute",
            post(governance::recompute::recompute_scores),
        )
        .route(
            "/governance/scores/{app_id}",
            get(governance::get_app_scores::get_app_scores),
        )
        .route(
            "/governance/patterns",
            get(governance::list_patterns::list_patterns),
        )
        // EU AI Act inventory
        .nest("/ai-act", ai_act::routes())
        // Run reconciliation
        .route("/runs/sweep", post(runs::sweep_runs))
        .route("/cache/sweep", post(cache::sweep_cache))
        // Fork orphan janitor
        .route("/forks/orphans", get(forks::list_orphan_forks))
        .route(
            "/forks/orphans/{app_id}/delete",
            post(forks::delete_orphan_fork),
        )
        // Logs / observability
        .route("/logs/errors", get(logs::list_errors::list_errors))
        .route("/logs/errors/{error_id}", get(logs::get_error::get_error))
        .route("/logs/stats", get(logs::stats::error_stats))
        .route("/logs/timeseries", get(logs::timeseries::error_timeseries))
        .route("/logs/chain-status", get(logs::chain_status::chain_status))
        // Telemetry dashboards
        .route("/telemetry/overview", get(telemetry::telemetry_overview))
        .route(
            "/telemetry/timeseries",
            get(telemetry::telemetry_timeseries),
        )
        .route("/telemetry/events", get(telemetry::list_telemetry_events))
        .route(
            "/telemetry/engagement",
            get(telemetry::telemetry_engagement),
        )
        .route("/telemetry/flowpilot", get(telemetry::telemetry_flowpilot))
        // Crash issues and release health
        .route(
            "/telemetry/issues",
            get(telemetry::issues::list_telemetry_issues),
        )
        .route(
            "/telemetry/issues/{issue_id}",
            get(telemetry::issues::get_telemetry_issue)
                .patch(telemetry::issues::update_telemetry_issue),
        )
        .route(
            "/telemetry/releases",
            get(telemetry::release_health::list_telemetry_releases),
        )
        .route(
            "/telemetry/release-health",
            get(telemetry::release_health::telemetry_release_health),
        )
        .route(
            "/telemetry/sourcemaps",
            post(telemetry::sourcemaps::upload_telemetry_sourcemap).layer(DefaultBodyLimit::max(
                telemetry::sourcemaps::SOURCE_MAP_BODY_LIMIT_BYTES,
            )),
        )
        // Distributed tracing, performance percentiles and retention
        .route(
            "/telemetry/traces",
            get(telemetry::traces::list_telemetry_traces),
        )
        .route(
            "/telemetry/traces/{trace_id}",
            get(telemetry::traces::get_telemetry_trace),
        )
        .route(
            "/telemetry/performance",
            get(telemetry::performance::telemetry_performance),
        )
        .route(
            "/telemetry/span-stats",
            get(telemetry::span_stats::telemetry_span_stats),
        )
        .route("/telemetry/sweep", post(telemetry::sweep::sweep_telemetry))
        .route(
            "/telemetry/rollup",
            post(telemetry::rollup::rollup_telemetry),
        )
        // LLM observability
        .route("/telemetry/llm", get(telemetry::llm::telemetry_llm))
        // Alert rules and the in-app alert inbox
        .route(
            "/telemetry/alerts",
            get(telemetry::alerts::list_telemetry_alert_rules)
                .post(telemetry::alerts::create_telemetry_alert_rule),
        )
        .route(
            "/telemetry/alerts/events",
            get(telemetry::alerts::list_telemetry_alert_events),
        )
        .route(
            "/telemetry/alerts/evaluate",
            post(telemetry::alerts::evaluate_telemetry_alerts),
        )
        .route(
            "/telemetry/alerts/{rule_id}",
            patch(telemetry::alerts::update_telemetry_alert_rule)
                .delete(telemetry::alerts::delete_telemetry_alert_rule),
        )
        .route(
            "/telemetry/alerts/{event_id}/ack",
            post(telemetry::alerts::acknowledge_telemetry_alert_event),
        )
        // Structured ad-hoc query builder, saved queries and dashboards
        .route(
            "/telemetry/query",
            post(telemetry::query::run_telemetry_query),
        )
        .route(
            "/telemetry/saved-queries",
            get(telemetry::saved_queries::list_telemetry_saved_queries)
                .post(telemetry::saved_queries::create_telemetry_saved_query),
        )
        .route(
            "/telemetry/saved-queries/{query_id}",
            patch(telemetry::saved_queries::update_telemetry_saved_query)
                .delete(telemetry::saved_queries::delete_telemetry_saved_query),
        )
        .route(
            "/telemetry/dashboards",
            get(telemetry::dashboards::list_telemetry_dashboards)
                .post(telemetry::dashboards::create_telemetry_dashboard),
        )
        .route(
            "/telemetry/dashboards/{dashboard_id}",
            patch(telemetry::dashboards::update_telemetry_dashboard)
                .delete(telemetry::dashboards::delete_telemetry_dashboard),
        )
}

#[cfg(test)]
mod tests {
    /// Router construction panics on a path conflict, so building the real
    /// admin router is the guard against a new route shadowing an existing one
    /// (the telemetry alert routes deliberately mix static segments with two
    /// differently named path parameters).
    #[test]
    fn the_admin_router_has_no_conflicting_routes() {
        let _ = super::routes();
    }
}
