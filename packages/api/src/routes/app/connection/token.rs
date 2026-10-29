use crate::{
    app_connection_jwt::{
        self, AppConnectionJwtParams, MAX_APP_CONNECTION_CHAIN, MAX_APP_CONNECTION_TTL_SECONDS,
    },
    entity::{app_connection, execution_run, sea_orm_active_enums::AppConnectionStatus},
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
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct CreateAppConnectionTokenRequest {
    /// Token lifetime in seconds, clamped to [60, 900]. Defaults to 600.
    pub ttl_seconds: Option<i64>,
    /// Run the token is minted for. Used when the caller authenticates with a
    /// user token (local/desktop executions) instead of an executor JWT: the
    /// run row — verified to be a run of this app the caller started — supplies
    /// the app chain and process correlation so cross-app cases stay connected.
    /// Any other run id is ignored. It never changes who the token names.
    #[serde(default)]
    pub run_id: Option<String>,
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
    description = "Mint a short-lived app-to-app token to call a connected app. The token pins both the origin and the target app and expires quickly. When a signed-in person started the run, the connected app acts on their behalf: with their own role there, limited by the connection's role, or only running its events when they are not a member. For a run on the hub that nobody started, such as a scheduled or webhook run, and for an anonymous visitor of a hosted page, the token names no person. A token minted with a person's own sign-in or personal access token, as runs on a device are, always names that person, whatever their role in the origin app; to act for the app without a person, run the flow on the hub. A claimed `run_id` counts only for a run of the origin app the caller started.",
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
        // only honored for a run of this app the caller started; the run
        // row is then the source of truth for chain and correlation.
        match claimed_run(&state.db, &app_id, &user, &claimed_run_id).await? {
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
                    "Claimed run id is not a run of this app the caller started — minting connection token without run correlation"
                );
                (None, Vec::new(), None)
            }
        }
    } else {
        (None, Vec::new(), None)
    };
    // The person who started the run is passed through the whole chain (A -> B -> C),
    // so downstream apps act for them even if they are not a member.
    let sub = connection_subject(&user, permission.effective_user_id().ok());
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

/// The run `run_id` the caller claimed, when it is a run of `app_id` the caller started: a
/// person's own run, or one their API key started. Any other run lends a token neither its id
/// nor its chain or case. A run on the hub claims nothing; its executor token names its run.
async fn claimed_run<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    user: &AppUser,
    run_id: &str,
) -> Result<Option<execution_run::Model>, DbErr> {
    let started_by_caller = match user {
        AppUser::OpenID(person) => execution_run::Column::UserId.eq(person.sub.as_str()),
        AppUser::PAT(person) => execution_run::Column::UserId.eq(person.sub.as_str()),
        AppUser::APIKey(key) => execution_run::Column::TechnicalUserId.eq(key.key_id.as_str()),
        AppUser::Executor(_) | AppUser::ConnectedApp(_) | AppUser::Unauthorized => {
            return Ok(None);
        }
    };
    execution_run::Entity::find_by_id(run_id)
        .filter(execution_run::Column::AppId.eq(app_id))
        .filter(started_by_caller)
        .one(db)
        .await
}

/// The subject a connection token passes on, which the connected app acts for when it
/// names an account. A run a person started passes that person, a signed-in visitor of a
/// hosted page included, and a person minting with their own sign-in or personal access
/// token is always named, whatever their role in the origin app: the connected app then
/// holds the call to their own standing there. Only a run on the hub that nobody started
/// passes a placeholder of the run, so the connected app never takes the user the event
/// runs as for a person; its signed executor token alone says so, never a request. An
/// anonymous visitor's run passes a pseudonym of the run, so the connected app never learns
/// who the visitor is.
fn connection_subject(user: &AppUser, effective_user_id: Option<String>) -> Option<String> {
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
    use crate::middleware::jwt::{ApiKey, ConnectedAppUser, ExecutorUser, OpenIDUser, PATUser};
    use crate::permission::role_permission::has_role_permission;

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
            let minted = connection_subject(&AppUser::Executor(executor), effective_user_id);
            assert_eq!(minted.as_deref(), Some(subject), "{case}");
            assert_eq!(person_in_target(minted).as_deref(), person, "{case}");
        }
        let minted = connection_subject(&AppUser::Executor(hosted(anonymous)), None).unwrap();
        assert_eq!(account_user_id(&minted), None);
        assert_eq!(
            connection_subject(&AppUser::Executor(run("user-1")), None),
            None
        );
        assert_eq!(
            connection_subject(&AppUser::Unauthorized, Some("user-1".into())).as_deref(),
            Some("user-1")
        );
    }

    fn desktop_user(sub: &str) -> AppUser {
        AppUser::OpenID(OpenIDUser {
            sub: sub.into(),
            access_token: "token".into(),
        })
    }

    fn pat_user(sub: &str) -> AppUser {
        AppUser::PAT(PATUser {
            pat: "pat_1.secret".into(),
            sub: sub.into(),
        })
    }

    fn api_key(key_id: &str) -> AppUser {
        AppUser::APIKey(ApiKey {
            key_id: key_id.into(),
            api_key: "secret".into(),
            app_id: "app-1".into(),
            creator_user_id: Some("user-1".into()),
        })
    }

    /// A request from a desktop that still says nobody started its run.
    fn still_declaring_unattended() -> CreateAppConnectionTokenRequest {
        serde_json::from_value(serde_json::json!({
            "run_id": "local-run",
            "ttl_seconds": 600,
            "unattended": true,
        }))
        .unwrap()
    }

    /// A desktop that still sends `unattended` is not refused, and the field is dropped as the
    /// request is read, so nothing of it is left for the handler to act on.
    #[test]
    fn a_request_still_declaring_a_run_unattended_is_read_without_it() {
        let plain: CreateAppConnectionTokenRequest =
            serde_json::from_value(serde_json::json!({"run_id": "local-run", "ttl_seconds": 600}))
                .unwrap();
        assert_eq!(
            format!("{:?}", still_declaring_unattended()),
            format!("{plain:?}")
        );
    }

    /// A person minting with their own sign-in or personal access token, as a run on a device
    /// does, is always named, so the connected app holds the call to their own standing there.
    /// Neither their role in the origin app, Owner included, nor a request that still says
    /// nobody started the run takes them off the token: the handler names the subject from
    /// the caller alone. Only a run on the hub acts for the app without a person.
    #[test]
    fn a_person_is_named_whatever_their_role_or_request() {
        let request = still_declaring_unattended();
        assert_eq!(request.run_id.as_deref(), Some("local-run"));
        let writer = RolePermissions::ExecuteEvents
            | RolePermissions::ExecuteBoards
            | RolePermissions::WriteEvents
            | RolePermissions::WriteBoards;
        for (case, role) in [
            ("runs events only", RolePermissions::ExecuteEvents),
            ("writes events and boards", writer),
            ("admin", RolePermissions::Admin),
            ("owner", RolePermissions::Owner),
        ] {
            assert!(
                has_role_permission(&role, RolePermissions::ExecuteEvents),
                "{case} may mint"
            );
            for user in [desktop_user("user-1"), pat_user("user-1")] {
                let subject = connection_subject(&user, user.effective_user_id().ok());
                assert_eq!(subject.as_deref(), Some("user-1"), "{case}");
                assert_eq!(
                    person_in_target(subject).as_deref(),
                    Some("user-1"),
                    "{case}"
                );
            }
        }

        let source = include_str!("token.rs");
        let handler = source
            .split_once("\npub async fn create_app_connection_token(")
            .and_then(|(_, rest)| rest.split_once("\n}\n"))
            .map(|(body, _)| body)
            .unwrap();
        assert!(
            handler.contains(
                "let sub = connection_subject(&user, permission.effective_user_id().ok());"
            ),
            "the subject comes from the caller alone, never from its role or its request"
        );
    }

    /// A run on the hub says whether a person started it only in its signed executor token:
    /// that claim alone takes the person off the token, and the placeholder names the signed
    /// run, never the run a request names or says nobody started.
    #[test]
    fn an_executor_claim_wins_over_its_request() {
        let request = still_declaring_unattended();
        assert_eq!(request.run_id.as_deref(), Some("local-run"));
        for (claimed, subject, person) in
            [(false, "user-1", Some("user-1")), (true, "run:run-1", None)]
        {
            let user = AppUser::Executor(ExecutorUser {
                unattended: claimed,
                ..run("user-1")
            });
            let minted = connection_subject(&user, user.effective_user_id().ok());
            assert_eq!(minted.as_deref(), Some(subject), "claimed {claimed}");
            assert_eq!(
                person_in_target(minted).as_deref(),
                person,
                "claimed {claimed}"
            );
        }
    }

    /// A SQLite stand-in for the runs a caller may claim.
    async fn runs() -> sea_orm::DatabaseConnection {
        use sea_orm::{
            ConnectOptions, Database, DatabaseBackend, Schema, Statement, sea_query::Table,
        };
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let source =
            Schema::new(db.get_database_backend()).create_table_from_entity(execution_run::Entity);
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
        let now = chrono::Utc::now().fixed_offset();
        for (id, app_id, user_id, key_id) in [
            ("person-run", "app-1", Some("user-1"), None),
            ("key-run", "app-1", Some("user-2"), Some("key-1")),
            ("other-app-run", "app-2", Some("user-1"), None),
        ] {
            db.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                r#"INSERT INTO public."ExecutionRun" (id,"boardId","logLevel","inputPayloadLen","outputPayloadLen",progress,"appId","userId","technicalUserId","createdAt","updatedAt",status,mode,"runVariant") VALUES (?,'board-1',0,0,0,0,?,?,?,?,?,'RUNNING','LOCAL','PRIMARY')"#,
                [
                    id.into(),
                    app_id.into(),
                    user_id.map(str::to_owned).into(),
                    key_id.map(str::to_owned).into(),
                    now.into(),
                    now.into(),
                ],
            ))
            .await
            .unwrap();
        }
        db
    }

    /// A claimed run lends a token its id, chain and case only when the caller started it in
    /// the origin app: a person their own runs, an API key the runs it started. Another
    /// person's or key's run, or a run of another app, is not found, and a run on the hub
    /// claims none.
    #[flow_like_types::tokio::test]
    async fn a_claimed_run_must_be_one_the_caller_started() {
        let db = runs().await;
        for (case, user, claimed, found) in [
            ("own run", desktop_user("user-1"), "person-run", true),
            ("own run with a PAT", pat_user("user-1"), "person-run", true),
            (
                "another person's run",
                desktop_user("user-3"),
                "person-run",
                false,
            ),
            (
                "another person's run with a PAT",
                pat_user("user-3"),
                "person-run",
                false,
            ),
            (
                "own run of another app",
                desktop_user("user-1"),
                "other-app-run",
                false,
            ),
            ("missing run", desktop_user("user-1"), "missing-run", false),
            ("the key's run", api_key("key-1"), "key-run", true),
            ("another key's run", api_key("key-2"), "key-run", false),
            (
                "a person's run with a key",
                api_key("key-1"),
                "person-run",
                false,
            ),
            (
                "a hub run",
                AppUser::Executor(run("user-1")),
                "person-run",
                false,
            ),
        ] {
            let honoured = claimed_run(&db, "app-1", &user, claimed).await.unwrap();
            assert_eq!(
                honoured.map(|row| row.id).as_deref(),
                found.then_some(claimed),
                "{case}"
            );
        }
    }
}
