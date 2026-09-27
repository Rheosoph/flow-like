use axum::{
    extract::{MatchedPath, Request},
    http::{HeaderValue, Method, header},
    middleware::Next,
    response::Response,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum CachePolicy {
    Unchanged,
    NoStore,
    OAuth,
}

fn cache_policy(method: &Method, route: Option<&str>) -> CachePolicy {
    let route = route.unwrap_or_default().trim_end_matches('/');
    match route {
        "/api/v1/auth/authorize"
        | "/api/v1/auth/token"
        | "/api/v1/auth/userinfo"
        | "/api/v1/auth/revoke"
        | "/api/v1/oauth/token/{provider_id}"
        | "/api/v1/oauth/refresh/{provider_id}"
        | "/api/v1/oauth/device/start/{provider_id}"
        | "/api/v1/oauth/device/poll/{provider_id}"
        | "/api/v1/oauth/userinfo/{provider_id}"
        | "/api/v1/oauth/revoke/{provider_id}" => CachePolicy::OAuth,
        "/api/v1/user/pat" | "/api/v1/apps/{app_id}/api" if method == Method::PUT => {
            CachePolicy::NoStore
        }
        "/api/v1/apps/{app_id}/data" | "/api/v1/apps/{app_id}/data/user"
            if method == Method::PUT =>
        {
            CachePolicy::NoStore
        }
        "/api/v1/apps/{app_id}/connections/{target_app_id}/token"
        | "/api/v1/apps/{app_id}/invoke/presign"
        | "/api/v1/apps/{app_id}/data/presign"
        | "/api/v1/apps/{app_id}/data/user/presign"
        | "/api/v1/apps/{app_id}/data/download"
        | "/api/v1/apps/{app_id}/data/user/download"
        | "/api/v1/apps/{app_id}/db/presign"
        | "/api/v1/apps/{app_id}/db/presign/project"
        | "/api/v1/tmp"
        | "/api/v1/tmp/batch"
        | "/api/v1/channels/{channel_id}/grant" => CachePolicy::NoStore,
        _ => CachePolicy::Unchanged,
    }
}

fn is_json(response: &Response) -> bool {
    let Some(content_type) = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let media_type = content_type.split(';').next().unwrap_or_default().trim();
    let Some((kind, subtype)) = media_type.split_once('/') else {
        return false;
    };
    kind.eq_ignore_ascii_case("application")
        && (subtype.eq_ignore_ascii_case("json")
            || subtype
                .rsplit_once('+')
                .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case("json")))
}

/// Wrap the standard API outside authentication and request decompression so
/// their rejections receive the same policy as handler and extractor errors.
/// User-authored REST/MCP and hosted documents use separate router boundaries.
pub async fn response_headers_middleware(request: Request, next: Next) -> Response {
    let policy = cache_policy(
        request.method(),
        request
            .extensions()
            .get::<MatchedPath>()
            .map(MatchedPath::as_str),
    );
    let mut response = next.run(request).await;

    if policy != CachePolicy::Unchanged {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    if policy == CachePolicy::OAuth {
        response
            .headers_mut()
            .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    }

    // Leave file and document MIME handling to their own responders. Adding a
    // default only here avoids changing executable assets or widget policies.
    if is_json(&response)
        || response.status().is_client_error()
        || response.status().is_server_error()
    {
        response
            .headers_mut()
            .entry(header::X_CONTENT_TYPE_OPTIONS)
            .or_insert(HeaderValue::from_static("nosniff"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        body::{Body, to_bytes},
        http::StatusCode,
        middleware::from_fn,
        response::IntoResponse,
        routing::{get, post, put},
    };
    use tower::ServiceExt;

    fn api(routes: Router) -> Router {
        Router::new().nest(
            "/api/v1",
            routes.layer(from_fn(response_headers_middleware)),
        )
    }

    fn request(method: Method, path: &str) -> Request {
        Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap()
    }

    async fn cached_json() -> impl IntoResponse {
        (
            [(header::CACHE_CONTROL, "public, max-age=600")],
            Json("value"),
        )
    }

    #[tokio::test]
    async fn oauth_headers_cover_nested_routes_and_replace_upstream_cache_policy() {
        let router = api(Router::new()
            .nest(
                "/oauth",
                Router::new().route("/token/{provider_id}", post(cached_json)),
            )
            .nest("/auth", Router::new().route("/token", post(cached_json))));
        for path in [
            "/api/v1/oauth/token/provider?code=ignored",
            "/api/v1/auth/token",
        ] {
            let response = router
                .clone()
                .oneshot(request(Method::POST, path))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(response.headers()[header::PRAGMA], "no-cache");
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
        }
    }

    #[tokio::test]
    async fn auth_rejections_keep_status_body_and_existing_headers() {
        async fn reject(_: Request, _: Next) -> Response {
            (
                StatusCode::UNAUTHORIZED,
                [
                    (header::WWW_AUTHENTICATE, "Bearer"),
                    (header::CONTENT_SECURITY_POLICY, "default-src 'none'"),
                ],
                "authentication failed",
            )
                .into_response()
        }
        let router = api(Router::new()
            .route("/oauth/refresh/{provider_id}", post(cached_json))
            .layer(from_fn(reject)));
        let response = router
            .oneshot(request(Method::POST, "/api/v1/oauth/refresh/provider"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()[header::PRAGMA], "no-cache");
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
        assert_eq!(
            response.headers()[header::CONTENT_SECURITY_POLICY],
            "default-src 'none'"
        );
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap(),
            "authentication failed"
        );
    }

    #[tokio::test]
    async fn extractor_rejections_and_handler_errors_receive_headers() {
        async fn json_input(Json(_): Json<String>) -> StatusCode {
            StatusCode::OK
        }
        let router = api(Router::new()
            .route("/oauth/device/poll/{provider_id}", post(json_input))
            .route(
                "/apps/{app_id}/invoke/presign",
                get(|| async { StatusCode::FORBIDDEN }),
            ));
        for (method, path, status) in [
            (
                Method::POST,
                "/api/v1/oauth/device/poll/provider",
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ),
            (
                Method::GET,
                "/api/v1/apps/app/invoke/presign",
                StatusCode::FORBIDDEN,
            ),
        ] {
            let response = router.clone().oneshot(request(method, path)).await.unwrap();
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
        }
    }

    #[tokio::test]
    async fn keys_and_storage_credentials_disable_caching_without_changing_public_json() {
        let router = api(Router::new()
            .route("/user/pat", put(cached_json).get(cached_json))
            .nest(
                "/apps/{app_id}/api",
                Router::new().route("/", put(cached_json)),
            )
            .route("/apps/{app_id}/data/user/presign", post(cached_json))
            .route("/apps/{app_id}/db/presign/project", post(cached_json))
            .route(
                "/apps/{app_id}/connections/{target_app_id}/token",
                post(cached_json),
            )
            .route("/tmp", get(cached_json))
            .route("/auth/jwks", get(cached_json)));
        for (method, path) in [
            (Method::PUT, "/api/v1/user/pat"),
            (Method::PUT, "/api/v1/apps/app/api"),
            (Method::POST, "/api/v1/apps/app/data/user/presign"),
            (Method::POST, "/api/v1/apps/app/db/presign/project"),
            (Method::POST, "/api/v1/apps/app/connections/target/token"),
            (Method::GET, "/api/v1/tmp"),
        ] {
            let response = router.clone().oneshot(request(method, path)).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "no-store",
                "{path}"
            );
            assert!(!response.headers().contains_key(header::PRAGMA));
        }
        for path in ["/api/v1/auth/jwks", "/api/v1/user/pat"] {
            let response = router
                .clone()
                .oneshot(request(Method::GET, path))
                .await
                .unwrap();
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "public, max-age=600"
            );
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
            assert!(!response.headers().contains_key(header::PRAGMA));
        }
    }

    #[tokio::test]
    async fn assets_widgets_and_inbound_responses_keep_their_policies() {
        async fn asset() -> impl IntoResponse {
            (
                [
                    (header::CONTENT_TYPE, "text/javascript"),
                    (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
                ],
                "script",
            )
        }
        async fn widget() -> impl IntoResponse {
            (
                [
                    (header::CONTENT_TYPE, "text/html"),
                    (header::CONTENT_SECURITY_POLICY, "sandbox allow-scripts"),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                "widget",
            )
        }
        let router = api(Router::new()
            .route("/asset.js", get(asset))
            .route("/widget", get(widget)))
        .nest("/r", Router::new().route("/custom", get(cached_json)))
        .nest("/m", Router::new().route("/custom", get(cached_json)));
        for path in ["/r/custom", "/m/custom", "/api/v1/asset.js"] {
            let response = router
                .clone()
                .oneshot(request(Method::GET, path))
                .await
                .unwrap();
            assert!(
                !response
                    .headers()
                    .contains_key(header::X_CONTENT_TYPE_OPTIONS)
            );
            assert!(!response.headers().contains_key(header::PRAGMA));
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .starts_with("public,")
            );
        }
        let response = router
            .oneshot(request(Method::GET, "/api/v1/widget"))
            .await
            .unwrap();
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response.headers()[header::CONTENT_SECURITY_POLICY],
            "sandbox allow-scripts"
        );
        assert_eq!(
            response
                .headers()
                .get_all(header::X_CONTENT_TYPE_OPTIONS)
                .iter()
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn json_media_types_and_errors_receive_only_a_default_nosniff() {
        let router = api(Router::new()
            .route(
                "/problem",
                get(|| async {
                    (
                        [(
                            header::CONTENT_TYPE,
                            "Application/Problem+JSON; charset=utf-8",
                        )],
                        "{}",
                    )
                }),
            )
            .route(
                "/custom",
                get(|| async {
                    (
                        StatusCode::BAD_REQUEST,
                        [(header::X_CONTENT_TYPE_OPTIONS, "existing-policy")],
                    )
                }),
            ));
        let response = router
            .clone()
            .oneshot(request(Method::GET, "/api/v1/problem"))
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert!(!response.headers().contains_key(header::CACHE_CONTROL));
        let response = router
            .oneshot(request(Method::GET, "/api/v1/custom"))
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "existing-policy"
        );
    }
}
