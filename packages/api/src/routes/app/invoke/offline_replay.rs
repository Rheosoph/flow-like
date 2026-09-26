use super::presign::invoke_access;
use crate::{
    credentials::CredentialsAccess,
    error::ApiError,
    instances::offline::{self, DesktopReplayPrincipal},
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use flow_like_device_protocol::{
    DESKTOP_OFFLINE_LIMITS, DesktopOfflineCapabilities, MAX_OFFLINE_REPLAY_HTTP_BYTES,
    OFFLINE_ERROR_FORBIDDEN, OFFLINE_ERROR_INVALID, OFFLINE_ERROR_LIMIT_EXCEEDED,
    OFFLINE_ERROR_PRINCIPAL_UNSUPPORTED, OFFLINE_ERROR_SUBJECT_MISMATCH,
    OFFLINE_INSTALLATION_HEADER, OFFLINE_SUBJECT_HEADER, OfflineContentProvider, OfflineLimits,
    OfflineReplayRequest, OfflineReplayResponse, OfflineResource, ProtocolError, StoragePurpose,
    validate_installation_id,
};
use std::sync::OnceLock;

const MAX_REQUEST_BYTES_ENV: &str = "FLOW_LIKE_OFFLINE_REPLAY_MAX_REQUEST_BYTES";
const MIN_REQUEST_BYTES: usize = 262_144;
const MAX_SUBJECT_BYTES: usize = 1024;
const CAPABILITIES_VERSION: u32 = 1;

/// Read once; deployments behind a proxy with a smaller body limit lower the request cap.
pub fn desktop_limits() -> OfflineLimits {
    static LIMITS: OnceLock<OfflineLimits> = OnceLock::new();
    *LIMITS
        .get_or_init(|| limits_with_override(std::env::var(MAX_REQUEST_BYTES_ENV).ok().as_deref()))
}

fn limits_with_override(value: Option<&str>) -> OfflineLimits {
    let mut limits = DESKTOP_OFFLINE_LIMITS;
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return limits;
    };
    match value.parse::<usize>() {
        Ok(bytes) => {
            limits.max_request_bytes =
                Some(bytes.clamp(MIN_REQUEST_BYTES, MAX_OFFLINE_REPLAY_HTTP_BYTES));
        }
        Err(_) => tracing::warn!(
            value,
            "{MAX_REQUEST_BYTES_ENV} is not a byte count; using the default desktop offline limit"
        ),
    }
    limits
}

fn offline_invalid(message: impl Into<String>) -> ApiError {
    ApiError::coded(StatusCode::BAD_REQUEST, OFFLINE_ERROR_INVALID, message)
}

pub(crate) fn desktop_forbidden() -> ApiError {
    ApiError::coded(
        StatusCode::FORBIDDEN,
        OFFLINE_ERROR_FORBIDDEN,
        "Your role in this project does not allow this offline change",
    )
}

/// Replay is allowed for what a desktop run may write through `invoke/presign`
/// (every invoke level writes the user prefix) and what Data Studio may write through `/db`.
/// `None` marks resources that `validate_desktop` rejects.
pub(crate) fn desktop_access(
    permissions: &RolePermissions,
    resource: &OfflineResource,
) -> Option<bool> {
    let invoke = invoke_access(permissions);
    let writes_tables = has_role_permission(permissions, RolePermissions::WriteFiles)
        || has_role_permission(permissions, RolePermissions::WriteDatabase);
    match resource {
        OfflineResource::Table {
            purpose: StoragePurpose::Storage,
            ..
        } => Some(writes_tables),
        OfflineResource::Table {
            purpose: StoragePurpose::User,
            ..
        } => Some(writes_tables || invoke.is_some()),
        OfflineResource::File {
            purpose: StoragePurpose::Files | StoragePurpose::Storage,
            ..
        } => Some(matches!(invoke, Some(CredentialsAccess::InvokeWrite))),
        OfflineResource::File {
            purpose: StoragePurpose::User,
            ..
        } => Some(invoke.is_some()),
        _ => None,
    }
}

/// A real account subject is needed to derive user paths and bind receipts.
fn desktop_subject(user: &AppUser) -> Result<String, ApiError> {
    match user {
        AppUser::OpenID(_) | AppUser::PAT(_) => user.sub(),
        _ => Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            OFFLINE_ERROR_PRINCIPAL_UNSUPPORTED,
            "Offline changes can only be replayed with a signed-in account or its personal access token",
        )),
    }
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

/// Returns the installation ID and the subject the device queued the change for.
fn offline_headers(headers: &HeaderMap) -> Result<(String, String), ApiError> {
    let installation = single_header(headers, OFFLINE_INSTALLATION_HEADER)
        .filter(|value| validate_installation_id(value).is_ok())
        .ok_or_else(|| offline_invalid("Offline installation identifier is missing or invalid"))?;
    let subject = single_header(headers, OFFLINE_SUBJECT_HEADER)
        .filter(|value| !value.is_empty() && value.len() <= MAX_SUBJECT_BYTES)
        .ok_or_else(|| offline_invalid("Offline subject is missing or invalid"))?;
    Ok((installation.into(), subject.into()))
}

/// Steps 4 to 6 of the desktop admission. Nothing here reads or claims a receipt.
fn admit_desktop(
    caller: &str,
    queued_subject: &str,
    permissions: &RolePermissions,
    request: &OfflineReplayRequest,
    limits: &OfflineLimits,
) -> Result<(), ApiError> {
    if queued_subject != caller {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            OFFLINE_ERROR_SUBJECT_MISMATCH,
            "These changes were queued for another account",
        ));
    }
    if desktop_access(permissions, &request.resource) == Some(false) {
        return Err(desktop_forbidden());
    }
    request
        .validate_desktop(limits)
        .map_err(|error| match error {
            ProtocolError::TooLarge { .. } => ApiError::coded(
                StatusCode::PAYLOAD_TOO_LARGE,
                OFFLINE_ERROR_LIMIT_EXCEEDED,
                "This change exceeds the desktop offline sync limit",
            ),
            other => offline_invalid(other.to_string()),
        })
}

fn capabilities_access(permissions: &RolePermissions) -> Result<(), ApiError> {
    invoke_access(permissions).map(|_| ()).ok_or_else(|| {
        ApiError::coded(
            StatusCode::FORBIDDEN,
            OFFLINE_ERROR_FORBIDDEN,
            "Your role in this project does not allow running it",
        )
    })
}

fn capabilities(provider: OfflineContentProvider) -> DesktopOfflineCapabilities {
    DesktopOfflineCapabilities {
        version: CAPABILITIES_VERSION,
        limits: desktop_limits(),
        provider,
    }
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/invoke/offline/replay",
    tag = "execution",
    description = "Apply one change that a desktop run queued while the hub was unreachable. Replays are idempotent per operation ID and never overwrite newer cloud data.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("x-flow-like-offline-installation" = String, Header, description = "Identifier of the desktop installation that queued the change"),
        ("x-flow-like-offline-subject" = String, Header, description = "Account the change was queued for; must be the caller")
    ),
    request_body(content = String, content_type = "application/json", description = "OfflineReplayRequest"),
    responses(
        (status = 200, description = "Replay outcome", body = String, content_type = "application/json"),
        (status = 400, description = "Invalid request or installation identifier"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The caller may not write this resource"),
        (status = 409, description = "Operation ID reused with another payload, or the change belongs to another account"),
        (status = 413, description = "The change exceeds the desktop offline sync limit")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/invoke/offline/replay",
    skip(state, user, headers, request)
)]
pub async fn offline_replay(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<OfflineReplayRequest>,
) -> Result<Json<OfflineReplayResponse>, ApiError> {
    let sub = desktop_subject(&user)?;
    let (installation_id, queued_subject) = offline_headers(&headers)?;
    let permission = user.app_permission_fresh(&app_id, &state).await?;
    admit_desktop(
        &sub,
        &queued_subject,
        &permission.permissions,
        &request,
        &desktop_limits(),
    )?;
    let principal = DesktopReplayPrincipal {
        sub,
        app_id,
        installation_id,
    };
    Ok(Json(
        offline::replay_desktop(&state, &user, principal, request).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/invoke/offline/capabilities",
    tag = "execution",
    description = "Report whether this hub accepts offline changes from desktop apps, with its size limits and storage provider.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "Offline capabilities", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The caller may not run this project")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/invoke/offline/capabilities",
    skip(state, user)
)]
pub async fn offline_capabilities(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<DesktopOfflineCapabilities>, ApiError> {
    desktop_subject(&user)?;
    let permission = user.app_permission_fresh(&app_id, &state).await?;
    capabilities_access(&permission.permissions)?;
    let master = state.master_credentials().await.map_err(|_| {
        ApiError::service_unavailable("Offline replay storage credentials are unavailable")
    })?;
    Ok(Json(capabilities(offline::content_provider(
        master.as_ref(),
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::jwt::{OpenIDUser, PATUser};
    use axum::http::HeaderValue;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use flow_like_device_protocol::{OfflineExpected, OfflineMutation, request_wire_bytes};
    use sha2::{Digest, Sha256};

    const INSTALLATION: &str = "71bfc449-a2f1-4fa1-aed8-a3819d0f32d5";

    fn file(purpose: StoragePurpose, bytes: &[u8]) -> OfflineReplayRequest {
        OfflineReplayRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            resource: OfflineResource::File {
                purpose,
                path: "report.txt".into(),
            },
            expected: OfflineExpected::FileAbsent,
            mutation: OfflineMutation::FilePut {
                data_base64: STANDARD.encode(bytes),
                sha256: format!("{:x}", Sha256::digest(bytes)),
            },
        }
    }

    fn table(purpose: StoragePurpose) -> OfflineReplayRequest {
        OfflineReplayRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            resource: OfflineResource::Table {
                purpose,
                database: "db".into(),
                table: "rows".into(),
            },
            expected: OfflineExpected::TableVersion {
                version: 1,
                fingerprint: Some(format!("blake3:{}", "0".repeat(64))),
            },
            mutation: OfflineMutation::TableInsert {
                rows: vec![serde_json::json!({"id": 1})],
            },
        }
    }

    fn resources() -> [(&'static str, OfflineResource); 5] {
        [
            ("project table", table(StoragePurpose::Storage).resource),
            ("user table", table(StoragePurpose::User).resource),
            ("upload file", file(StoragePurpose::Files, b"x").resource),
            ("storage file", file(StoragePurpose::Storage, b"x").resource),
            ("user file", file(StoragePurpose::User, b"x").resource),
        ]
    }

    fn code(error: &ApiError) -> (u16, &str) {
        (error.status().as_u16(), error.public_code())
    }

    fn headers(installation: Option<&str>, subject: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(value) = installation {
            headers.insert(
                OFFLINE_INSTALLATION_HEADER,
                HeaderValue::from_str(value).unwrap(),
            );
        }
        if let Some(value) = subject {
            headers.insert(
                OFFLINE_SUBJECT_HEADER,
                HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn desktop_permission_matrix_matches_presign_policies() {
        let run = RolePermissions::ExecuteEvents;
        let levels = [
            ("InvokeWrite", run | RolePermissions::WriteFiles, true),
            ("InvokeRead", run | RolePermissions::ReadFiles, false),
            ("InvokeNone", run, false),
        ];
        for (level, permissions, writes_project_files) in levels {
            assert!(invoke_access(&permissions).is_some(), "{level}");
            for (name, resource) in resources() {
                let expected = match &resource {
                    OfflineResource::Table {
                        purpose: StoragePurpose::Storage,
                        ..
                    } => permissions.contains(RolePermissions::WriteFiles),
                    OfflineResource::File {
                        purpose: StoragePurpose::Files | StoragePurpose::Storage,
                        ..
                    } => writes_project_files,
                    _ => true,
                };
                assert_eq!(
                    desktop_access(&permissions, &resource),
                    Some(expected),
                    "{level}: {name}"
                );
            }
        }
        assert!(matches!(
            invoke_access(&(run | RolePermissions::WriteFiles)),
            Some(CredentialsAccess::InvokeWrite)
        ));
        assert!(matches!(
            invoke_access(&(run | RolePermissions::ReadFiles)),
            Some(CredentialsAccess::InvokeRead)
        ));
        assert!(matches!(
            invoke_access(&run),
            Some(CredentialsAccess::InvokeNone)
        ));

        let data_studio = RolePermissions::WriteDatabase;
        assert!(invoke_access(&data_studio).is_none());
        for (name, resource) in resources() {
            let expected = matches!(resource, OfflineResource::Table { .. });
            assert_eq!(
                desktop_access(&data_studio, &resource),
                Some(expected),
                "WriteDatabase without ExecuteEvents: {name}"
            );
        }
        for (name, resource) in resources() {
            assert_eq!(
                desktop_access(&RolePermissions::empty(), &resource),
                Some(false),
                "no role: {name}"
            );
            for admin in [RolePermissions::Owner, RolePermissions::Admin] {
                assert_eq!(desktop_access(&admin, &resource), Some(true), "{name}");
            }
        }
        let mut temporary = file(StoragePurpose::Temporary, b"x");
        assert_eq!(
            desktop_access(&RolePermissions::Owner, &temporary.resource),
            None
        );
        temporary.resource = OfflineResource::Table {
            purpose: StoragePurpose::Files,
            database: "db".into(),
            table: "rows".into(),
        };
        assert_eq!(
            desktop_access(&RolePermissions::Owner, &temporary.resource),
            None
        );
    }

    #[test]
    fn installation_header_is_required_and_canonical() {
        for installation in [
            None,
            Some(""),
            Some("71BFC449-A2F1-4FA1-AED8-A3819D0F32D5"),
            Some("71bfc449-a2f1-1fa1-aed8-a3819d0f32d5"),
            Some("71bfc449a2f14fa1aed8a3819d0f32d5"),
        ] {
            let error = offline_headers(&headers(installation, Some("user"))).unwrap_err();
            assert_eq!(
                code(&error),
                (400, OFFLINE_ERROR_INVALID),
                "{installation:?}"
            );
            assert_eq!(
                error.public_message(),
                Some("Offline installation identifier is missing or invalid")
            );
        }
        let mut duplicated = headers(Some(INSTALLATION), Some("user"));
        duplicated.append(
            OFFLINE_INSTALLATION_HEADER,
            HeaderValue::from_static(INSTALLATION),
        );
        assert_eq!(
            code(&offline_headers(&duplicated).unwrap_err()),
            (400, OFFLINE_ERROR_INVALID)
        );
        for subject in [None, Some("")] {
            assert_eq!(
                code(&offline_headers(&headers(Some(INSTALLATION), subject)).unwrap_err()),
                (400, OFFLINE_ERROR_INVALID)
            );
        }
        assert_eq!(
            offline_headers(&headers(Some(INSTALLATION), Some("auth0|user"))).unwrap(),
            (INSTALLATION.to_string(), "auth0|user".to_string())
        );
    }

    #[test]
    fn subject_header_mismatch_is_409_with_code() {
        let request = file(StoragePurpose::User, b"x");
        let error = admit_desktop(
            "caller",
            "someone-else",
            &RolePermissions::Owner,
            &request,
            &DESKTOP_OFFLINE_LIMITS,
        )
        .unwrap_err();
        assert_eq!(code(&error), (409, OFFLINE_ERROR_SUBJECT_MISMATCH));
        assert_eq!(
            error.public_message(),
            Some("These changes were queued for another account")
        );
        admit_desktop(
            "caller",
            "caller",
            &RolePermissions::ExecuteEvents,
            &request,
            &DESKTOP_OFFLINE_LIMITS,
        )
        .unwrap();
    }

    #[test]
    fn unsupported_principals_are_rejected_with_code() {
        let error = desktop_subject(&AppUser::Unauthorized).unwrap_err();
        assert_eq!(code(&error), (403, OFFLINE_ERROR_PRINCIPAL_UNSUPPORTED));
        assert_eq!(
            desktop_subject(&AppUser::OpenID(OpenIDUser {
                sub: "account".into(),
                access_token: "token".into(),
            }))
            .unwrap(),
            "account"
        );
        assert_eq!(
            desktop_subject(&AppUser::PAT(PATUser {
                pat: "pat_token".into(),
                sub: "owner".into(),
            }))
            .unwrap(),
            "owner"
        );
    }

    #[test]
    fn validate_desktop_enforces_limits_rejects_temporary_and_maps_413() {
        let admit = |request: &OfflineReplayRequest, limits: &OfflineLimits| {
            admit_desktop("caller", "caller", &RolePermissions::Owner, request, limits)
        };
        let limits = OfflineLimits {
            max_operation_bytes: 64,
            max_file_bytes: 4,
            max_request_bytes: None,
        };
        admit(&file(StoragePurpose::Files, b"four"), &limits).unwrap();
        let error = admit(&file(StoragePurpose::Files, b"fives"), &limits).unwrap_err();
        assert_eq!(code(&error), (413, OFFLINE_ERROR_LIMIT_EXCEEDED));
        assert_eq!(
            error.public_message(),
            Some("This change exceeds the desktop offline sync limit")
        );

        let request = file(StoragePurpose::User, b"payload");
        let wire = request_wire_bytes(&request).unwrap();
        let exact = OfflineLimits {
            max_request_bytes: Some(wire),
            ..DESKTOP_OFFLINE_LIMITS
        };
        admit(&request, &exact).unwrap();
        let below = OfflineLimits {
            max_request_bytes: Some(wire - 1),
            ..DESKTOP_OFFLINE_LIMITS
        };
        assert_eq!(
            code(&admit(&request, &below).unwrap_err()),
            (413, OFFLINE_ERROR_LIMIT_EXCEEDED)
        );

        let error = admit(
            &file(StoragePurpose::Temporary, b"x"),
            &DESKTOP_OFFLINE_LIMITS,
        )
        .unwrap_err();
        assert_eq!(code(&error), (400, OFFLINE_ERROR_INVALID));
        let mut invalid = table(StoragePurpose::Storage);
        invalid.operation_id = "not-a-uuid".into();
        assert_eq!(
            code(&admit(&invalid, &DESKTOP_OFFLINE_LIMITS).unwrap_err()),
            (400, OFFLINE_ERROR_INVALID)
        );
    }

    #[test]
    fn desktop_admission_rejects_before_any_receipt() {
        let denied = admit_desktop(
            "caller",
            "caller",
            &RolePermissions::ExecuteEvents,
            &file(StoragePurpose::Files, b"x"),
            &DESKTOP_OFFLINE_LIMITS,
        )
        .unwrap_err();
        assert_eq!(code(&denied), (403, OFFLINE_ERROR_FORBIDDEN));
        let readonly = admit_desktop(
            "caller",
            "caller",
            &(RolePermissions::ExecuteEvents | RolePermissions::ReadFiles),
            &table(StoragePurpose::Storage),
            &DESKTOP_OFFLINE_LIMITS,
        )
        .unwrap_err();
        assert_eq!(code(&readonly), (403, OFFLINE_ERROR_FORBIDDEN));
    }

    #[test]
    fn env_override_clamps_max_request_bytes() {
        assert_eq!(limits_with_override(None), DESKTOP_OFFLINE_LIMITS);
        assert_eq!(limits_with_override(Some(" ")), DESKTOP_OFFLINE_LIMITS);
        assert_eq!(
            limits_with_override(Some("garbage")),
            DESKTOP_OFFLINE_LIMITS
        );
        assert_eq!(
            limits_with_override(Some("900000")).max_request_bytes,
            Some(900_000)
        );
        assert_eq!(
            limits_with_override(Some("1")).max_request_bytes,
            Some(MIN_REQUEST_BYTES)
        );
        assert_eq!(
            limits_with_override(Some("999999999999")).max_request_bytes,
            Some(MAX_OFFLINE_REPLAY_HTTP_BYTES)
        );
        let lowered = limits_with_override(Some("1000000"));
        assert_eq!(
            lowered.max_operation_bytes,
            DESKTOP_OFFLINE_LIMITS.max_operation_bytes
        );
        assert_eq!(
            lowered.max_file_bytes,
            DESKTOP_OFFLINE_LIMITS.max_file_bytes
        );
    }

    #[test]
    fn capabilities_report_limits_and_provider() {
        let value = serde_json::to_value(capabilities(OfflineContentProvider::Az)).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["provider"], "az");
        assert_eq!(
            value["limits"]["maxRequestBytes"],
            serde_json::json!(desktop_limits().max_request_bytes)
        );
        assert_eq!(
            value["limits"]["maxFileBytes"],
            DESKTOP_OFFLINE_LIMITS.max_file_bytes
        );
        #[cfg(feature = "aws")]
        assert_eq!(
            offline::content_provider(&crate::credentials::RuntimeCredentials::Aws(
                crate::credentials::aws_credentials::AwsRuntimeCredentials::new(
                    "meta",
                    "content",
                    "logs",
                    "us-east-1",
                ),
            )),
            OfflineContentProvider::S3
        );
    }

    #[test]
    fn capabilities_require_execute_events() {
        for permissions in [
            RolePermissions::empty(),
            RolePermissions::WriteFiles | RolePermissions::WriteDatabase,
        ] {
            assert_eq!(
                code(&capabilities_access(&permissions).unwrap_err()),
                (403, OFFLINE_ERROR_FORBIDDEN)
            );
        }
        for permissions in [
            RolePermissions::ExecuteEvents,
            RolePermissions::Owner,
            RolePermissions::Admin,
        ] {
            capabilities_access(&permissions).unwrap();
        }
    }
}
