//! Submit review for a package

use crate::error::ApiError;
use crate::mail::{EmailMessage, templates::package_review_update};
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::permission::wasm_package_permission::WasmPackagePermission;
use crate::routes::registry::server::{PackageReview, ReviewRequest};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[utoipa::path(
    post,
    path = "/admin/packages/{package_id}/review",
    tag = "admin",
    params(
        ("package_id" = String, Path, description = "Package ID to review")
    ),
    request_body = ReviewRequest,
    responses(
        (status = 200, description = "Review submitted", body = PackageReview),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
pub async fn review_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
    Json(review): Json<ReviewRequest>,
) -> Result<Json<PackageReview>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ManagePackages)
        .await?;

    let sub = user.sub()?;

    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("WASM registry not configured"))?;

    let review_action = review.action.clone();
    let review_comment = review.comment.clone();

    let result = registry.submit_review(&package_id, &sub, review).await?;

    // Notify package owner(s) via email
    if let Some(mail_client) = &state.mail_client {
        use crate::entity::{user, wasm_package, wasm_package_user};

        let frontend_url = std::env::var("FRONTEND_URL")
            .unwrap_or_else(|_| "https://app.flow-like.com".to_string());
        let package_url = format!("{}/nodes?id={}", frontend_url, package_id);

        let pkg_name = wasm_package::Entity::find_by_id(&package_id)
            .one(&state.db)
            .await
            .ok()
            .flatten()
            .map(|p| p.name)
            .unwrap_or_else(|| package_id.clone());

        let owners = wasm_package_user::Entity::find()
            .filter(wasm_package_user::Column::PackageId.eq(&package_id))
            .filter(wasm_package_user::Column::Permission.eq(WasmPackagePermission::Owner.bits()))
            .all(&state.db)
            .await
            .unwrap_or_default();

        for owner in owners {
            let email_addr = user::Entity::find_by_id(&owner.user_id)
                .one(&state.db)
                .await
                .ok()
                .flatten()
                .and_then(|u| u.email);

            if let Some(addr) = email_addr {
                let (html, text) = package_review_update(
                    &pkg_name,
                    &package_url,
                    &review_action,
                    review_comment.as_deref(),
                );

                let email = EmailMessage {
                    to: addr,
                    subject: format!("Package Review: {} — {}", pkg_name, review_action),
                    body_html: Some(html),
                    body_text: Some(text),
                };

                if let Err(e) = mail_client.send(email).await {
                    tracing::warn!(error = %e, "Failed to send package review email");
                }
            }
        }
    }

    Ok(Json(result))
}
