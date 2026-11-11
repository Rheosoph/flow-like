//! A bot requests context from a person in its current conversation. The inbox validates the
//! person's reply; neither a bot token nor its owner's account can answer for that person.

use async_trait::async_trait;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_bot_tools::user_context::{
    UserContextAvailability, UserContextBackend, UserContextParticipant, UserContextRequestArgs,
    UserContextRequestReceipt,
};
use flow_like_types::{
    Result,
    bot_user_context::{UserContextAudience, UserContextField, UserContextRequest},
    create_id,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Deserialize;
use serde_json::json;
use utoipa::IntoParams;

use crate::{
    bot_caller::{self, Names},
    entity::{bot, conversation, conversation_member, inbox_entry},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::{
        self,
        exchange::{message_outgoing, post_as_bot},
    },
    state::AppState,
};

const MAX_REQUESTS_PER_SUBJECT_HOUR: usize = 6;
const REQUEST_WINDOW_MS: i64 = 3_600_000;

/// The bot's own request cards for `subject_id` in the conversation during the last hour.
pub(crate) async fn recent_requests<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    subject_id: &str,
    now: i64,
) -> Result<usize, sea_orm::DbErr> {
    let since = chrono::DateTime::from_timestamp_millis(now - REQUEST_WINDOW_MS)
        .unwrap_or_default()
        .fixed_offset();
    let rows = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::SenderId.eq(bot_id))
        .filter(inbox_entry::Column::Kind.eq("MESSAGE"))
        .filter(inbox_entry::Column::CreatedAt.gt(since))
        .order_by_desc(inbox_entry::Column::CreatedAt)
        .limit(200)
        .all(db)
        .await?;
    Ok(rows
        .iter()
        .filter_map(|row| row.envelope["body"].get("user_context_request"))
        .filter(|card| card["subject_id"].as_str() == Some(subject_id))
        .count())
}

#[derive(Deserialize, IntoParams)]
pub struct ContextQuery {
    pub conversation_id: String,
}

#[derive(Deserialize)]
pub struct ContextRequest {
    pub conversation_id: String,
    #[serde(flatten)]
    pub request: UserContextRequestArgs,
}

pub(crate) struct ServerUserContext {
    state: AppState,
    bot_id: String,
    conversation_id: Option<String>,
}

impl ServerUserContext {
    pub(crate) fn new(state: &AppState, bot_id: &str, conversation_id: Option<&str>) -> Self {
        Self {
            state: state.clone(),
            bot_id: bot_id.to_owned(),
            conversation_id: conversation_id.map(str::to_owned),
        }
    }

    async fn conversation(&self) -> Result<(conversation::Model, Vec<conversation_member::Model>)> {
        let id = self.conversation_id.as_deref().ok_or_else(|| {
            backend_error(ApiError::bad_request(
                "User context requires an active conversation",
            ))
        })?;
        let bot = bot::Entity::find_by_id(&self.bot_id)
            .filter(bot::Column::DeletedAt.is_null())
            .one(&self.state.db)
            .await?
            .ok_or_else(|| backend_error(ApiError::not_found("This bot is no longer available")))?;
        if !bot
            .capabilities
            .iter()
            .any(|capability| capability == "user_context")
        {
            return Err(backend_error(ApiError::bad_request(
                "The bot's User context capability is off",
            )));
        }
        inbox::active_membership(&self.state.db, id, &self.bot_id)
            .await
            .map_err(backend_error)?;
        let conversation = conversation::Entity::find_by_id(id)
            .one(&self.state.db)
            .await?
            .ok_or_else(|| {
                backend_error(ApiError::not_found(
                    "This conversation is no longer available",
                ))
            })?;
        if !conversation.encryption.eq_ignore_ascii_case("plain") {
            return Err(backend_error(ApiError::bad_request(
                "User context requests require a plain conversation",
            )));
        }
        let members = inbox::active_members(&self.state.db, &[id.to_owned()]).await?;
        Ok((conversation, members))
    }
}

#[async_trait]
impl UserContextBackend for ServerUserContext {
    async fn available(&self) -> Result<UserContextAvailability> {
        let (_, members) = self.conversation().await?;
        let ids: Vec<_> = members
            .iter()
            .filter(|member| member.principal_kind.eq_ignore_ascii_case("user"))
            .map(|member| member.principal_id.clone())
            .collect();
        let mut names = inbox::outreach::person_names(&self.state.db, &ids).await?;
        Ok(UserContextAvailability {
            participants: ids
                .into_iter()
                .map(|id| UserContextParticipant {
                    name: names.remove(&id).unwrap_or_else(|| id.clone()),
                    id,
                })
                .collect(),
            fields: UserContextField::ALL.to_vec(),
            device_support: "unknown_until_shared".into(),
        })
    }

    async fn request(&self, args: UserContextRequestArgs) -> Result<UserContextRequestReceipt> {
        let (conversation, members) = self.conversation().await?;
        if !members.iter().any(|member| {
            member.principal_id == args.subject_id
                && member.principal_kind.eq_ignore_ascii_case("user")
        }) {
            return Err(backend_error(ApiError::bad_request(
                "Choose a person who is currently in this conversation",
            )));
        }
        let now = chrono::Utc::now().timestamp_millis();
        let sent = recent_requests(
            &self.state.db,
            &self.bot_id,
            &conversation.id,
            &args.subject_id,
            now,
        )
        .await?;
        if sent >= MAX_REQUESTS_PER_SUBJECT_HOUR {
            return Err(backend_error(ApiError::too_many_requests(format!(
                "You already asked this person for context {sent} times in the last hour; wait before asking again"
            ))));
        }
        let id = create_id();
        let mut audience: Vec<_> = members
            .iter()
            .map(|member| member.principal_id.clone())
            .collect();
        audience.sort();
        let request = UserContextRequest::new(
            id.clone(),
            self.bot_id.clone(),
            conversation.id.clone(),
            args,
            if conversation.kind.eq_ignore_ascii_case("group") {
                UserContextAudience::Group
            } else {
                UserContextAudience::Direct
            },
            audience,
            now,
        );
        request
            .validate(now)
            .map_err(ApiError::bad_request)
            .map_err(backend_error)?;
        let receipt = UserContextRequestReceipt {
            request_id: id.clone(),
            conversation_id: conversation.id.clone(),
            subject_id: request.subject_id.clone(),
            expires_at: request.expires_at,
        };
        let outgoing = message_outgoing(
            &id,
            json!({
                "text": "Please choose whether to share the requested context.",
                "user_context_request": request,
            }),
        )?;
        post_as_bot(&self.state, &conversation.id, &self.bot_id, 0, outgoing).await?;
        Ok(receipt)
    }
}

// Hosted tools display anyhow errors directly, so keep the public message outside the typed error.
fn backend_error(error: ApiError) -> flow_like_types::Error {
    let message = error.public_message().map(str::to_owned);
    let error = flow_like_types::Error::new(error);
    match message {
        Some(message) => error.context(message),
        None => error,
    }
}

// The tool backend uses anyhow; restore the API's status and public message at the HTTP boundary.
fn context_error(error: flow_like_types::Error) -> ApiError {
    let error = match error.downcast::<ApiError>() {
        Ok(error) => return error,
        Err(error) => error,
    };
    match error.downcast::<sea_orm::DbErr>() {
        Ok(error) => error.into(),
        Err(error) => ApiError::internal_error(error),
    }
}

async fn authorized<C: ConnectionTrait>(
    db: &C,
    user: &AppUser,
    bot_id: &str,
    room: &str,
) -> Result<(), ApiError> {
    let owner = bot_caller::acting_owner(
        db,
        user,
        &Names {
            as_bot: Some(bot_id),
            conversation: Some(room),
            turn: None,
        },
    )
    .await?;
    super::owned_bot(db, &owner, bot_id).await?;
    if !matches!(user, AppUser::Bot(_)) {
        inbox::active_membership(db, room, &owner).await?;
    }
    if let AppUser::Bot(token) = user
        && token.conv_id.is_none()
    {
        let turn = crate::entity::bot_turn::Entity::find_by_id(&token.turn_id)
            .one(db)
            .await?
            .filter(|turn| turn.bot_id == bot_id && turn.owner_id == owner)
            .ok_or_else(|| {
                ApiError::forbidden("User context requires this bot's conversation turn")
            })?;
        let origin = crate::bot_host::talk::turn_origin_conversation(db, &turn).await?;
        if origin != room {
            return Err(ApiError::forbidden(
                "User context belongs to the bot's current conversation",
            ));
        }
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/user-context/available",
    tag = "Bots",
    description = "The people in a conversation your bot can ask to share context, and the fields it can ask for. You must be in the conversation, and the bot must have the User context capability on.",
    params(("bot_id" = String, Path, description = "Your bot's id"), ContextQuery),
    responses(
        (status = 200, description = "The participants the bot can ask, the shareable fields and device support", body = Object),
        (status = 400, description = "The conversation or the bot cannot request user context"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "The bot's turn does not belong to this conversation"),
        (status = 404, description = "You have no bot with this id, or you are not in the conversation")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/user-context/available", skip_all)]
pub async fn available(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Query(query): Query<ContextQuery>,
) -> Result<Json<UserContextAvailability>, ApiError> {
    authorized(&state.db, &user, &bot_id, &query.conversation_id).await?;
    ServerUserContext::new(&state, &bot_id, Some(&query.conversation_id))
        .available()
        .await
        .map(Json)
        .map_err(context_error)
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/user-context/request",
    tag = "Bots",
    description = "Ask a person in the conversation to share context such as their location or time zone. The person decides in the conversation whether to share; nobody else can answer for them.",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body(content = Object, description = "The conversation_id, the person's subject_id, the requested fields, the purpose and optionally the precision"),
    responses(
        (status = 200, description = "A receipt for the request posted to the conversation", body = Object),
        (status = 400, description = "The request is invalid, or the person is not in the conversation"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "The bot's turn does not belong to this conversation"),
        (status = 404, description = "You have no bot with this id, or you are not in the conversation")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/user-context/request", skip_all)]
pub async fn request(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<ContextRequest>,
) -> Result<Json<UserContextRequestReceipt>, ApiError> {
    authorized(&state.db, &user, &bot_id, &body.conversation_id).await?;
    ServerUserContext::new(&state, &bot_id, Some(&body.conversation_id))
        .request(body.request)
        .await
        .map(Json)
        .map_err(context_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::jwt::{BotUser, OpenIDUser};
    use axum::http::StatusCode;

    #[tokio::test]
    async fn missing_context_conversation_preserves_its_status_and_message() {
        let db = crate::routes::inbox::fixture::database().await;
        let missing = inbox::active_membership(&db, "bot:bot_test", "bot_test")
            .await
            .unwrap_err();
        let expected_message = missing.public_message().unwrap().to_owned();
        let missing = backend_error(missing);
        assert_eq!(missing.to_string(), expected_message);
        let error = context_error(missing.context("listing context"));

        assert_eq!(error.status(), StatusCode::NOT_FOUND);
        assert_eq!(error.public_code(), "NOT_FOUND");
        assert_eq!(error.public_message(), Some(expected_message.as_str()));
    }

    #[test]
    fn invalid_context_requests_remain_bad_requests() {
        let request = UserContextRequest::new(
            "request".into(),
            "bot".into(),
            "room".into(),
            UserContextRequestArgs {
                subject_id: "person".into(),
                fields: vec![],
                purpose: "Arrange a meeting".into(),
                precision: Default::default(),
                status_change: None,
            },
            UserContextAudience::Direct,
            vec!["bot".into(), "person".into()],
            1_000,
        );
        let invalid = request
            .validate(1_000)
            .map_err(ApiError::bad_request)
            .map_err(backend_error)
            .unwrap_err();
        assert_eq!(
            invalid.to_string(),
            "Choose distinct context fields and describe their purpose"
        );
        let error = context_error(invalid);

        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            error.public_message(),
            Some("Choose distinct context fields and describe their purpose")
        );
    }

    #[test]
    fn missing_context_stays_readable_to_hosted_tools() {
        let error = backend_error(ApiError::not_found(
            "This conversation is no longer available",
        ));
        assert_eq!(
            error.to_string(),
            "This conversation is no longer available"
        );
        let error = context_error(error);
        assert_eq!(error.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            error.public_message(),
            Some("This conversation is no longer available")
        );
    }

    #[test]
    fn context_database_failures_keep_server_statuses_and_hide_details() {
        let error = context_error(sea_orm::DbErr::Custom("private database details".into()).into());
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.public_code(), "DATABASE_ERROR");
        assert_eq!(error.public_message(), None);

        let error = context_error(
            sea_orm::DbErr::ConnectionAcquire(sea_orm::ConnAcquireErr::Timeout).into(),
        );
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.public_code(), "DATABASE_UNAVAILABLE");
        assert_eq!(
            error.public_message(),
            Some("The database is busy; retry shortly.")
        );
    }

    #[test]
    fn remote_host_request_uses_the_same_scoped_arguments() {
        let request: ContextRequest = serde_json::from_value(json!({
            "conversation_id": "room", "subject_id": "person", "fields": ["location", "timezone"],
            "purpose": "Find a meeting place",
        }))
        .unwrap();
        assert_eq!(request.conversation_id, "room");
        assert_eq!(request.request.subject_id, "person");
        assert_eq!(
            request.request.precision,
            flow_like_types::bot_user_context::UserContextPrecision::Approximate
        );
        assert!(
            serde_json::from_value::<ContextRequest>(json!({
                "conversation_id": "room", "subject_id": "person", "fields": ["location"],
                "purpose": "Find a meeting place", "audience_member_ids": ["someone-else"],
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn request_routes_bind_bot_tokens_to_their_bot_and_conversation() {
        let db = crate::routes::inbox::fixture::database().await;
        crate::routes::inbox::fixture::insert_bot(&db, "bot_test", "owner", "DESKTOP").await;
        crate::routes::inbox::fixture::running_turn(
            &db,
            "turn_test",
            "bot_test",
            "room",
            "host",
            crate::routes::inbox::fixture::at(60),
        )
        .await;
        let token = AppUser::Bot(BotUser {
            bot_id: "bot_test".into(),
            turn_id: "turn_test".into(),
            conv_id: Some("room".into()),
        });
        assert!(authorized(&db, &token, "bot_test", "room").await.is_ok());
        assert!(
            authorized(&db, &token, "bot_test", "other-room")
                .await
                .is_err()
        );
        assert!(authorized(&db, &token, "other-bot", "room").await.is_err());
        let other = AppUser::OpenID(OpenIDUser {
            sub: "other-person".into(),
            access_token: String::new(),
        });
        assert!(authorized(&db, &other, "bot_test", "room").await.is_err());
        let owner = AppUser::OpenID(OpenIDUser {
            sub: "owner".into(),
            access_token: String::new(),
        });
        assert!(authorized(&db, &owner, "bot_test", "room").await.is_err());
        db.execute_unprepared(r#"INSERT INTO public."ConversationMember" ("conversationId","principalId","principalKind","joinedAt","status") VALUES ('room','owner','USER','2023-01-01T00:00:00+00:00','ACTIVE')"#).await.unwrap();
        assert!(authorized(&db, &owner, "bot_test", "room").await.is_ok());
        assert!(
            authorized(&db, &owner, "bot_test", "other-room")
                .await
                .is_err()
        );
        let direct = AppUser::Bot(BotUser {
            bot_id: "bot_test".into(),
            turn_id: "direct:app".into(),
            conv_id: None,
        });
        assert!(authorized(&db, &direct, "bot_test", "room").await.is_err());
        let unbound = AppUser::Bot(BotUser {
            bot_id: "bot_test".into(),
            turn_id: "missing-turn".into(),
            conv_id: None,
        });
        assert!(authorized(&db, &unbound, "bot_test", "room").await.is_err());
    }
}
