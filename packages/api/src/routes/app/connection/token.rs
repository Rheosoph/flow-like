use crate::{
    app_connection_jwt::{
        self, AppConnectionJwtParams, MAX_APP_CONNECTION_CHAIN, MAX_APP_CONNECTION_TTL_SECONDS,
    },
    entity::{app_connection, sea_orm_active_enums::AppConnectionStatus},
    error::ApiError,
    execution::subject::{
        account_user_id, hosted_connection_subject, unattended_connection_subject,
    },
    middleware::jwt::{AppUser, ExecutorUse},
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct CreateAppConnectionTokenRequest {
    /// Token lifetime in seconds, clamped to [60, 900]. Defaults to 600.
    pub ttl_seconds: Option<i64>,
    /// Run the token is minted for. Used when the caller authenticates with a
    /// user token (local/desktop executions) instead of an executor JWT: the
    /// run row — verified to belong to this app — supplies the app chain and
    /// process correlation so cross-app cases stay connected.
    #[serde(default)]
    pub run_id: Option<String>,
    /// No person started the run, such as a schedule on a desktop that mints with its user's
    /// own token: the token then names no person, only a placeholder of the run, and the
    /// connected app treats the call like one from a run nobody triggered. It can only take
    /// away the person the token would pass on.
    #[serde(default)]
    pub unattended: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateAppConnectionTokenResponse {
    /// Short-lived JWT to call the target app. Bound to origin and target app.
    pub token: String,
    pub origin_app_id: String,
    pub target_app_id: String,
    /// Expiration as Unix timestamp (seconds)
    pub expires_at: i64,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/connections/{target_app_id}/token",
    tag = "team",
    description = "Mint a short-lived app-to-app token to call a connected app. The token pins both the origin and the target app and expires quickly. When a signed-in person started the run, the connected app acts on their behalf. For a run nobody started, such as a scheduled or webhook run, and for an anonymous visitor of a hosted page, the token names no person. A run on a device that mints with its user's own sign-in says it was not started by a person with `unattended: true`.",
    params(
        ("app_id" = String, Path, description = "Application ID (the origin app)"),
        ("target_app_id" = String, Path, description = "The connected app to call")
    ),
    request_body = CreateAppConnectionTokenRequest,
    responses(
        (status = 200, description = "App connection token", body = CreateAppConnectionTokenResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: execution permission required, or the run is a regression replay"),
        (status = 404, description = "No active connection to the target app")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/connections/{target_app_id}/token",
    skip(state, user, payload)
)]
pub async fn create_app_connection_token(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, target_app_id)): Path<(String, String)>,
    payload: Option<Json<CreateAppConnectionTokenRequest>>,
) -> Result<Json<CreateAppConnectionTokenResponse>, ApiError> {
    // App-connection tokens must not be exchangeable for further
    // app-connection tokens; that would allow hopping across apps.
    if matches!(user, AppUser::ConnectedApp(_)) {
        return Err(ApiError::forbidden(
            "App connection tokens cannot mint further app connection tokens",
        ));
    }

    // Minting happens during flow execution (desktop: user token, cloud:
    // executor JWT, headless: API key) — any principal allowed to execute in
    // the origin app may act as the app.
    let permission = user
        .execution_app_permission(&app_id, &state, ExecutorUse::Write("call connected apps"))
        .await?;
    if !permission.has_permission(RolePermissions::ExecuteBoards)
        && !permission.has_permission(RolePermissions::ExecuteEvents)
    {
        return Err(ApiError::FORBIDDEN);
    }

    let connection = app_connection::Entity::find()
        .filter(
            app_connection::Column::SourceAppId
                .eq(&app_id)
                .and(app_connection::Column::TargetAppId.eq(&target_app_id))
                .and(app_connection::Column::Status.eq(AppConnectionStatus::Active)),
        )
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("No active connection to the target app"))?;

    if connection.role_id.is_none() {
        return Err(ApiError::forbidden(
            "The connection has no role assigned yet",
        ));
    }

    // Extend the app chain: if this run itself was triggered through an app
    // connection, its executor JWT carries the upstream chain.
    let (run_id, mut app_chain, correlation) = if let AppUser::Executor(executor) = &user {
        (
            Some(executor.run_id.clone()),
            executor.app_chain.clone().unwrap_or_default(),
            executor.correlation.clone(),
        )
    } else if let Some(claimed_run_id) = payload.as_ref().and_then(|p| p.run_id.clone()) {
        // Local/desktop executions authenticate with the user's token, so the
        // run context can't come from an executor JWT. The claimed run id is
        // only honored after verifying the run belongs to this app; the run
        // row is then the source of truth for chain and correlation.
        use crate::entity::execution_run;
        match execution_run::Entity::find_by_id(&claimed_run_id)
            .filter(execution_run::Column::AppId.eq(&app_id))
            .one(&state.db)
            .await?
        {
            Some(run) => {
                let keys = run
                    .correlation_keys
                    .as_ref()
                    .and_then(|value| serde_json::from_value(value.clone()).ok())
                    .unwrap_or_default();
                let correlation = crate::correlation::CorrelationContext {
                    trace_id: run.trace_id.clone().or_else(|| Some(run.id.clone())),
                    keys,
                };
                (
                    Some(run.id),
                    run.caller_app_chain.unwrap_or_default().into(),
                    Some(correlation),
                )
            }
            None => {
                // Don't reject: fully-local runs may not have a server-side
                // row until report_run, and the token itself stays valid —
                // only the process-case linkage is lost. Warn so broken
                // correlation is diagnosable instead of silent.
                tracing::warn!(
                    run_id = %claimed_run_id,
                    app_id = %app_id,
                    "Claimed run id not found for this app — minting connection token without run correlation"
                );
                (None, Vec::new(), None)
            }
        }
    } else {
        (None, Vec::new(), None)
    };
    // The person who started the run is passed through the whole chain (A -> B -> C),
    // so downstream apps act for them even if they are not a member.
    let unattended = unattended_run(payload.as_deref(), run_id.as_deref());
    let sub = connection_subject(&user, permission.effective_user_id().ok(), unattended);
    // Reject cycles (A -> B -> A) before extending the chain: revisiting an app
    // already in the chain would let calls loop indefinitely across apps.
    if app_chain.contains(&app_id) {
        return Err(ApiError::forbidden("App connection chain contains a cycle"));
    }
    app_chain.push(app_id.clone());

    if app_chain.len() > MAX_APP_CONNECTION_CHAIN {
        return Err(ApiError::forbidden("The app connection chain is too deep"));
    }

    let ttl_seconds = payload
        .as_ref()
        .and_then(|p| p.ttl_seconds)
        .unwrap_or(600)
        .clamp(60, MAX_APP_CONNECTION_TTL_SECONDS);

    let token = app_connection_jwt::sign(AppConnectionJwtParams {
        sub,
        origin_app_id: app_id.clone(),
        target_app_id: target_app_id.clone(),
        app_chain,
        technical_user_id: permission.technical_user_id().map(|id| id.to_string()),
        run_id,
        correlation,
        ttl_seconds: Some(ttl_seconds),
    })
    .map_err(|err| {
        tracing::error!("Failed to sign app connection token: {}", err);
        ApiError::internal("Failed to sign app connection token")
    })?;

    Ok(Json(CreateAppConnectionTokenResponse {
        token,
        origin_app_id: app_id,
        target_app_id,
        expires_at: chrono::Utc::now().timestamp() + ttl_seconds,
    }))
}

/// The run a caller declared that nobody started, named by the id the server holds for it,
/// `run_id`, or else a fresh one; never by an id the caller claimed without proof. `None`
/// when the caller declared nothing.
fn unattended_run(
    request: Option<&CreateAppConnectionTokenRequest>,
    run_id: Option<&str>,
) -> Option<String> {
    request
        .filter(|request| request.unattended)
        .map(|_| run_id.map_or_else(flow_like_types::create_id, str::to_owned))
}

/// The subject a connection token passes on, which the connected app acts for when it
/// names an account. A run a person started passes that person, a signed-in visitor of a
/// hosted page included. A run nobody started passes a placeholder of the run, so the
/// connected app never takes the user the event runs as for a person: its executor token
/// says so, or the caller does with `declared_unattended`, the run it mints for, which can
/// only take a person away. An anonymous visitor's run passes a pseudonym of the run, so the
/// connected app never learns who the visitor is.
fn connection_subject(
    user: &AppUser,
    effective_user_id: Option<String>,
    declared_unattended: Option<String>,
) -> Option<String> {
    if let Some(run_id) = declared_unattended {
        return Some(unattended_connection_subject(&run_id));
    }
    let AppUser::Executor(executor) = user else {
        return effective_user_id;
    };
    if executor.unattended {
        return Some(unattended_connection_subject(&executor.run_id));
    }
    if executor.hosted_frontend && account_user_id(&executor.sub).is_none() {
        return Some(hosted_connection_subject(
            executor.event_id.as_deref().unwrap_or_default(),
            &executor.run_id,
        ));
    }
    effective_user_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::jwt::{ConnectedAppUser, ExecutorUser};

    fn run(sub: &str) -> ExecutorUser {
        ExecutorUser {
            sub: sub.into(),
            app_id: "app-1".into(),
            run_id: "run-1".into(),
            technical_user_id: None,
            app_chain: None,
            correlation: None,
            shadow: false,
            hosted_frontend: false,
            event_id: Some("event-1".into()),
            custom_bits_owner: None,
            unattended: false,
        }
    }

    /// The person the connected app acts for, read from a token minted with `subject`.
    fn person_in_target(subject: Option<String>) -> Option<String> {
        let target = ConnectedAppUser {
            sub: subject,
            origin_app_id: "app-1".into(),
            target_app_id: "app-2".into(),
            app_chain: vec!["app-1".into()],
            technical_user_id: None,
            run_id: Some("run-1".into()),
            correlation: None,
        };
        target.person().map(str::to_owned)
    }

    /// Who each kind of run passes to the app it calls: a person who started it is passed
    /// through, a signed-in visitor of a hosted page included; a run nobody started passes a
    /// placeholder and an anonymous visitor's run a pseudonym, each per run and naming nobody.
    #[test]
    fn connection_tokens_pass_on_only_the_person_who_started_the_run() {
        let anonymous = "frontend:event-1:anonymous-session-1";
        let unattended = |sub: &str| ExecutorUser {
            unattended: true,
            ..run(sub)
        };
        let hosted = |sub: &str| ExecutorUser {
            hosted_frontend: true,
            ..run(sub)
        };
        for (case, executor, subject, person) in [
            ("person", run("user-1"), "user-1", Some("user-1")),
            (
                "signed-in visitor",
                hosted("visitor-1"),
                "visitor-1",
                Some("visitor-1"),
            ),
            (
                "anonymous visitor",
                hosted(anonymous),
                "frontend:event-1:run-run-1",
                None,
            ),
            (
                "run-as of a fire",
                unattended("run-as-1"),
                "run:run-1",
                None,
            ),
            (
                "fire without run-as",
                unattended("sink:sink-1"),
                "run:run-1",
                None,
            ),
        ] {
            let effective_user_id = Some(executor.sub.clone());
            let minted = connection_subject(&AppUser::Executor(executor), effective_user_id, None);
            assert_eq!(minted.as_deref(), Some(subject), "{case}");
            assert_eq!(person_in_target(minted).as_deref(), person, "{case}");
        }
        let minted = connection_subject(&AppUser::Executor(hosted(anonymous)), None, None).unwrap();
        assert_eq!(account_user_id(&minted), None);
        assert_eq!(
            connection_subject(&AppUser::Executor(run("user-1")), None, None),
            None
        );
        assert_eq!(
            connection_subject(&AppUser::Unauthorized, Some("user-1".into()), None).as_deref(),
            Some("user-1")
        );
    }

    /// A run that says nobody started it, such as a desktop schedule minting with its user's
    /// own token, passes no person whoever the caller is; the flag can only take a person
    /// away, never name one.
    #[test]
    fn a_run_declared_unattended_passes_no_person() {
        let desktop_user = AppUser::OpenID(crate::middleware::jwt::OpenIDUser {
            sub: "user-1".into(),
            access_token: "token".into(),
        });
        let pat_user = AppUser::PAT(crate::middleware::jwt::PATUser {
            pat: "pat_1.secret".into(),
            sub: "user-1".into(),
        });
        for (case, user) in [
            ("desktop session", desktop_user),
            ("trigger token", pat_user),
            ("person's executor", AppUser::Executor(run("user-1"))),
            (
                "unattended executor",
                AppUser::Executor(ExecutorUser {
                    unattended: true,
                    ..run("run-as-1")
                }),
            ),
        ] {
            let minted = connection_subject(&user, Some("user-1".into()), Some("local-run".into()));
            assert_eq!(minted.as_deref(), Some("run:local-run"), "{case}");
            assert_eq!(person_in_target(minted), None, "{case}");
        }

        let request: CreateAppConnectionTokenRequest =
            serde_json::from_value(serde_json::json!({"run_id": "local-run"})).unwrap();
        assert!(!request.unattended, "a person's run by default");
        let request: CreateAppConnectionTokenRequest =
            serde_json::from_value(serde_json::json!({"unattended": true})).unwrap();
        assert!(request.unattended);
    }

    /// The placeholder names only a run the server knows, or a fresh id: never an unverified
    /// run id the caller claimed.
    #[test]
    fn the_placeholder_never_carries_an_unverified_run_id() {
        let declared = CreateAppConnectionTokenRequest {
            unattended: true,
            run_id: Some("claimed-run".into()),
            ..Default::default()
        };
        assert_eq!(
            unattended_run(Some(&declared), Some("verified-run")).as_deref(),
            Some("verified-run")
        );
        let fresh = unattended_run(Some(&declared), None).unwrap();
        assert!(!fresh.is_empty() && fresh != "claimed-run");
        let person = CreateAppConnectionTokenRequest {
            run_id: Some("verified-run".into()),
            ..Default::default()
        };
        assert_eq!(unattended_run(Some(&person), Some("verified-run")), None);
        assert_eq!(unattended_run(None, Some("verified-run")), None);

        let source = include_str!("token.rs");
        let handler = source
            .split_once("\npub async fn create_app_connection_token(")
            .and_then(|(_, rest)| rest.split_once("\n}\n"))
            .map(|(body, _)| body)
            .unwrap();
        let chain = handler
            .find("let (run_id, mut app_chain, correlation)")
            .unwrap();
        let minted = handler
            .find("unattended_run(payload.as_deref(), run_id.as_deref())")
            .unwrap();
        assert!(chain < minted);
    }
}
