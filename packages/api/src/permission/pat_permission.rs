/// Access levels shared with the token creation UI. These values are not flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i64)]
pub enum PatPermission {
    ReadOnly = 1,
    ReadWrite = 2,
    Admin = 4,
}

pub(crate) const LEGACY_PERMISSION_ERROR: &str = "This token has an unsupported legacy permission value. Create a replacement token with Read Only, Read & Write, or Admin access.";

impl PatPermission {
    pub const fn from_bits(value: i64) -> Option<Self> {
        match value {
            1 => Some(Self::ReadOnly),
            2 => Some(Self::ReadWrite),
            4 => Some(Self::Admin),
            _ => None,
        }
    }

    pub const fn bits(self) -> i64 {
        self as i64
    }
}

pub fn has_pat_permission(permissions: &PatPermission, permission: PatPermission) -> bool {
    *permissions >= permission
}

/// This ceiling supplements the handler's current user and app permissions.
/// Admin tokens never create authority that the account itself does not have.
pub(crate) fn permits_route(bits: i64, method: &str, path: &str) -> bool {
    let Some(permission) = PatPermission::from_bits(bits) else {
        return false;
    };
    permission >= required_permission(method, path)
}

fn required_permission(method: &str, path: &str) -> PatPermission {
    use PatPermission::{Admin, ReadOnly, ReadWrite};
    let path = path.strip_prefix("/api/v1/").unwrap_or(path);
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    let read = matches!(method, "GET" | "HEAD" | "OPTIONS");

    // These operations edit content or consume access already offered to the
    // caller. Adjacent routes that grant access to others remain Admin-only.
    if matches!(
        (method, parts.as_slice()),
        ("PUT", ["apps", _, "connections", "notes"])
            | ("PUT" | "DELETE", ["apps", _, "connections", "notes", _])
            | (
                "PUT" | "DELETE",
                ["apps", _, "connections", _, "ontologies", _, "install"]
            )
            | ("PUT", ["apps", _, "team", "queue"])
            | ("POST", ["apps", _, "team", "purchase"])
            | ("POST", ["apps", _, "team", "link", "join", _])
            | ("POST" | "DELETE", ["user", "invites", _])
            | ("PUT", ["registry", "package", _, "access"])
            | ("POST", ["registry", "invitation", _, "accept" | "reject"])
            | ("POST", ["sink", _, "toggle"])
    ) {
        return ReadWrite;
    }

    // These routes manage platform authority or issue account/device credentials.
    // Read-only metadata in ordinary app routes remains available below.
    match parts.as_slice() {
        ["admin" | "maintenance" | "devices" | "instances", ..] => return Admin,
        ["auth", "discovery" | "jwks" | "openid" | "userinfo"] => {}
        ["auth" | "oauth", ..] => return Admin,
        ["user", "billing"] => return Admin, // Creates a billing-portal session.
        ["apps", _, "team", "link", ..] => return Admin, // Includes invitation secrets.
        ["apps", _, "events", _, "teams", "access"] => return Admin,
        ["sink", "configs" | "schedules", ..] => return Admin,
        ["sink", "trigger", ..] => return ReadWrite,
        _ => {}
    }

    if !read {
        match parts.as_slice() {
            ["user", "pat" | "invites", ..]
            | ["user", "payments", "connect", ..]
            | ["apps", _, "api" | "roles" | "team" | "visibility", ..]
            | ["apps", _, "connections", ..]
            | ["apps", _, "audit", "webhook", ..]
            | ["apps", _, "events", _, "teams" | "registrations", ..]
            | ["apps", _, "marketplace", "comp" | "requests", ..]
            | ["registry", "package", _, "users" | "access", ..]
            | ["registry", "invitation", ..]
            | ["sink", _, ..] => return Admin,
            _ => {}
        }
    }

    // Some GET handlers issue write-capable credentials or start work. HEAD
    // runs the same handlers, so it must have the same restriction.
    match parts.as_slice() {
        ["tmp" | "uploads", ..]
        | ["solution", "upload"]
        | ["apps", _, "invoke", "presign"]
        | ["apps", _, "events", _, "email-address"]
        | ["apps", _, "events", _, "rest" | "mcp", ..]
        | ["channels", _, "grant"]
        | ["courses", _, "translate"]
        | ["sink", "trigger", ..] => return ReadWrite,
        _ => {}
    }

    if read {
        return ReadOnly;
    }

    // Verified POST reads include registry inspection and GET-only download
    // signatures. Download counters do not change the caller's authority.
    // SQL and saved-query endpoints deliberately stay ReadWrite.
    if method == "POST"
        && matches!(
            parts.as_slice(),
            ["user", "lookup"]
                | ["auth", "userinfo"]
                | ["apps", _, "data", "list" | "download"]
                | ["apps", _, "data", "user", "list" | "download"]
                | [
                    "registry",
                    "check-id" | "check-version" | "hash-check" | "prerun-check" | "download"
                ]
        )
    {
        return ReadOnly;
    }
    ReadWrite
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_allows_inspection_but_not_writes_or_execution() {
        for path in [
            "/api/v1/user/info",
            "/user/pat",
            "/apps/a/roles",
            "/apps/a/board/b",
        ] {
            assert!(permits_route(1, "GET", path), "{path}");
            for method in ["PUT", "PATCH", "DELETE"] {
                assert!(!permits_route(1, method, path), "{method} {path}");
            }
        }
        for path in [
            "/tmp",
            "/solution/upload",
            "/uploads/file",
            "/apps/a/invoke/presign",
            "/channels/c/grant",
            "/apps/a/events/e/rest/resource",
            "/apps/a/events/e/mcp",
            "/courses/c/translate",
            "/sink/trigger/http/a/path",
            "/user/billing",
            "/auth/authorize",
            "/auth/revoke",
            "/apps/a/team/link",
            "/apps/a/events/e/teams/access",
        ] {
            for method in ["GET", "HEAD"] {
                assert!(!permits_route(1, method, path), "{method} {path}");
            }
        }
        for path in [
            "/apps/a/events/e/invoke",
            "/apps/a/db/t/query",
            "/apps/a/db/queries/execute",
        ] {
            assert!(!permits_route(1, "POST", path), "{path}");
        }
    }

    #[test]
    fn read_write_edits_content_but_cannot_delegate_or_change_access() {
        for (method, path) in [
            ("PUT", "/apps/a/board/b"),
            ("PATCH", "/apps/a/settings/forking"),
            ("POST", "/apps/a/events/e/invoke"),
            ("POST", "/apps/a/data/upload"),
            ("GET", "/apps/a/invoke/presign"),
            ("POST", "/sink/trigger/async"),
            ("GET", "/apps/a/team"),
            ("GET", "/apps/a/connections"),
            ("GET", "/user/info"),
        ] {
            assert!(permits_route(2, method, path), "{method} {path}");
        }
        for (method, path) in [
            ("PUT", "/user/pat"),
            ("DELETE", "/user/pat/id"),
            ("PUT", "/apps/a/api"),
            ("POST", "/apps/a/roles/r/assign/u"),
            ("PUT", "/apps/a/team/invite"),
            ("PATCH", "/apps/a/visibility"),
            ("POST", "/apps/a/connections/b/token"),
            ("PATCH", "/registry/package/p/users/u"),
            ("GET", "/devices"),
            ("GET", "/admin/users"),
            ("POST", "/oauth/token/provider"),
        ] {
            assert!(!permits_route(2, method, path), "{method} {path}");
            assert!(permits_route(4, method, path));
        }
    }

    #[test]
    fn post_read_exceptions_are_exact_and_do_not_include_sql() {
        for path in [
            "/user/lookup",
            "/apps/a/data/list",
            "/apps/a/data/download",
            "/apps/a/data/user/list",
            "/apps/a/data/user/download",
            "/registry/check-id",
            "/registry/check-version",
            "/registry/hash-check",
            "/registry/prerun-check",
            "/registry/download",
        ] {
            assert!(permits_route(1, "POST", path), "{path}");
            assert!(!permits_route(1, "POST", &format!("{path}/extra")));
        }
        assert!(!permits_route(1, "POST", "/apps/a/db/t/query"));
    }

    #[flow_like_types::tokio::test]
    async fn registry_read_posts_dispatch_with_the_original_api_prefix() {
        use axum::{
            Router,
            body::Body,
            extract::{OriginalUri, Request},
            http::StatusCode,
            middleware::{Next, from_fn},
            response::Response,
            routing::post,
        };
        use tower::ServiceExt;

        async fn read_only(
            OriginalUri(uri): OriginalUri,
            request: Request,
            next: Next,
        ) -> Response {
            if !permits_route(1, request.method().as_str(), uri.path()) {
                return Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(Body::empty())
                    .unwrap();
            }
            next.run(request).await
        }

        let mut registry = Router::new();
        for path in [
            "/check-id",
            "/check-version",
            "/hash-check",
            "/prerun-check",
            "/download",
            "/publish",
            "/upload-url",
        ] {
            registry = registry.route(path, post(|| async { StatusCode::NO_CONTENT }));
        }
        let app = Router::new().nest(
            "/api/v1",
            Router::new()
                .nest("/registry", registry)
                .layer(from_fn(read_only)),
        );
        for (path, status) in [
            ("check-id", StatusCode::NO_CONTENT),
            ("check-version", StatusCode::NO_CONTENT),
            ("hash-check", StatusCode::NO_CONTENT),
            ("prerun-check", StatusCode::NO_CONTENT),
            ("download", StatusCode::NO_CONTENT),
            ("publish", StatusCode::FORBIDDEN),
            ("upload-url", StatusCode::FORBIDDEN),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/api/v1/registry/{path}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{path}");
        }
    }

    #[test]
    fn read_write_preserves_content_and_self_service_routes() {
        for (method, path) in [
            ("PUT", "/apps/a/connections/notes"),
            ("PUT", "/apps/a/connections/notes/n"),
            ("DELETE", "/apps/a/connections/notes/n"),
            ("PUT", "/apps/a/connections/b/ontologies/o/install"),
            ("DELETE", "/apps/a/connections/b/ontologies/o/install"),
            ("PUT", "/apps/a/team/queue"),
            ("POST", "/apps/a/team/purchase"),
            ("POST", "/apps/a/team/link/join/token"),
            ("POST", "/user/invites/i"),
            ("DELETE", "/user/invites/i"),
            ("PUT", "/registry/package/p/access"),
            ("POST", "/registry/invitation/i/accept"),
            ("POST", "/registry/invitation/i/reject"),
            ("POST", "/sink/e/toggle"),
            ("POST", "/apps/a/groups"),
            ("PUT", "/apps/a/groups/g"),
            ("PATCH", "/apps/a/groups/g/visibility"),
            ("POST", "/apps/a/groups/g/members"),
            ("POST", "/apps/a/groups/requests/m"),
        ] {
            for full_path in [
                path.to_string(),
                format!("/api/v1{path}"),
                format!("/api/v1{path}/"),
            ] {
                assert!(permits_route(2, method, &full_path), "{method} {full_path}");
                assert!(
                    !permits_route(1, method, &full_path),
                    "{method} {full_path}"
                );
            }
        }
        for (method, path) in [
            ("POST", "/apps/a/team/queue/request"),
            ("PUT", "/apps/a/team/link"),
            ("DELETE", "/apps/a/team/member"),
            ("PUT", "/apps/a/connections/b"),
            ("POST", "/apps/a/connections/b/token"),
            ("POST", "/registry/package/p/access/request"),
            ("PATCH", "/sink/e"),
        ] {
            assert!(!permits_route(2, method, path), "{method} {path}");
            assert!(permits_route(4, method, path), "{method} {path}");
        }
    }

    #[test]
    fn legacy_capability_combinations_are_not_access_levels() {
        for bits in [0, -1, 3, 5, 6, 7, 8, 16, 32, 63, 65, 1 << 62] {
            assert!(!permits_route(bits, "GET", "/user/info"));
        }
        assert_eq!(PatPermission::from_bits(1), Some(PatPermission::ReadOnly));
        assert_eq!(PatPermission::from_bits(2), Some(PatPermission::ReadWrite));
        assert_eq!(PatPermission::from_bits(4), Some(PatPermission::Admin));
    }

    #[flow_like_types::tokio::test]
    async fn admin_token_still_needs_the_callers_app_permission() {
        use crate::{
            error::ApiError,
            permission::role_permission::{RolePermissions, has_role_permission},
        };

        struct CurrentRole(RolePermissions);
        impl CurrentRole {
            async fn app_permission_fresh(&self, _: &str, _: &()) -> Result<Self, ApiError> {
                Ok(Self(self.0))
            }
            fn has_permission(&self, required: RolePermissions) -> bool {
                has_role_permission(&self.0, required)
            }
        }
        async fn edit(role: CurrentRole) -> Result<(), ApiError> {
            assert!(permits_route(4, "PUT", "/apps/a/board/b"));
            // Exercise the handler's real guard after the PAT ceiling passes.
            crate::ensure_fresh_permission!(role, "a", &(), RolePermissions::WriteBoards);
            Ok(())
        }
        assert!(
            edit(CurrentRole(RolePermissions::ReadBoards))
                .await
                .is_err()
        );
        assert!(
            edit(CurrentRole(RolePermissions::WriteBoards))
                .await
                .is_ok()
        );
    }
}
