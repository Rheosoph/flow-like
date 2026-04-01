use axum::{
    Router,
    routing::{get, patch, post},
};

use crate::state::AppState;

pub mod dashboard;
pub mod discounts;
pub mod overview;
pub mod price;
pub mod purchases;
pub mod update_aggregations;

pub fn routes() -> Router<AppState> {
    Router::new()
        // Sales overview/stats
        .route("/", get(overview::get_sales_overview))
        .route("/stats", get(overview::get_sales_stats))
        // Purchases list
        .route("/purchases", get(purchases::list_purchases))
        // Price management
        .route("/price", patch(price::update_price))
        // Discounts CRUD
        .route(
            "/discounts",
            get(discounts::list_discounts).post(discounts::create_discount),
        )
        .route(
            "/discounts/{discount_id}",
            get(discounts::get_discount)
                .patch(discounts::update_discount)
                .delete(discounts::delete_discount),
        )
        .route(
            "/discounts/{discount_id}/toggle",
            post(discounts::toggle_discount),
        )
        .route("/dashboard", get(dashboard::dashboard))
}
