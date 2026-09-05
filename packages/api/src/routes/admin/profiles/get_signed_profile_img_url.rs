use std::time::Duration;

use crate::{
    error::ApiError, middleware::jwt::AppUser, permission::global_permission::GlobalPermission,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use flow_like_types::create_id;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct SignedProfileImgUrl {
    pub url: String,
    pub final_url: Option<String>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct ProfileMediaQuery {
    format: Option<String>,
}

fn media_extension(format: Option<&str>) -> Result<&'static str, ApiError> {
    match format.unwrap_or("webp") {
        "webp" => Ok("webp"),
        "png" => Ok("png"),
        "jpeg" => Ok("jpg"),
        _ => Err(ApiError::bad_request(
            "Profile images must use webp, png or jpeg format.",
        )),
    }
}

#[tracing::instrument(name = "GET /admin/profiles/media", skip(state, user))]
pub async fn get_signed_profile_img_url(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ProfileMediaQuery>,
) -> Result<Json<SignedProfileImgUrl>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteProfile)
        .await?;
    let extension = media_extension(query.format.as_deref())?;

    let id = create_id();
    let cdn_bucket = state.cdn_bucket.clone();
    let path = flow_like_storage::object_store::path::Path::from("profiles")
        .child(format!("{id}.{extension}"));

    let url = cdn_bucket
        .sign("PUT", &path, Duration::from_secs(60 * 60))
        .await?;
    let final_url = state
        .platform_config
        .cdn
        .as_ref()
        .filter(|url| !url.trim().is_empty())
        .map(|url| format!("{}/{}", url.trim_end_matches('/'), path));
    let signed_url = SignedProfileImgUrl {
        url: url.to_string(),
        final_url,
    };

    Ok(Json(signed_url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_format_defaults_to_webp_and_accepts_browser_fallback_formats() {
        assert_eq!(media_extension(None).unwrap(), "webp");
        assert_eq!(media_extension(Some("webp")).unwrap(), "webp");
        assert_eq!(media_extension(Some("png")).unwrap(), "png");
        assert_eq!(media_extension(Some("jpeg")).unwrap(), "jpg");
        for invalid in ["", "svg", "gif", "../png", "image/png"] {
            assert_eq!(
                media_extension(Some(invalid)).unwrap_err().status(),
                axum::http::StatusCode::BAD_REQUEST
            );
        }
    }
}
