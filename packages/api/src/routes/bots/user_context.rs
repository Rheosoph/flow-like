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
    Result, anyhow,
    bot_user_context::{UserContextAudience, UserContextField, UserContextRequest},
    create_id,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use serde_json::json;

use crate::{
    bot_caller::{self, Names},
    entity::{bot, conversation, conversation_member},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::{
        self,
        exchange::{message_outgoing, post_as_bot},
    },
    state::AppState,
};

#[derive(Deserialize)]
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
        let id = self
            .conversation_id
            .as_deref()
            .ok_or_else(|| anyhow!("User context requires an active conversation"))?;
        let bot = bot::Entity::find_by_id(&self.bot_id)
            .filter(bot::Column::DeletedAt.is_null())
            .one(&self.state.db)
            .await?
            .ok_or_else(|| anyhow!("This bot is no longer available"))?;
        if !bot
            .capabilities
            .iter()
            .any(|capability| capability == "user_context")
        {
            return Err(anyhow!("The bot's User context capability is off"));
        }
        inbox::active_membership(&self.state.db, id, &self.bot_id).await?;
        let conversation = conversation::Entity::find_by_id(id)
            .one(&self.state.db)
            .await?
            .ok_or_else(|| anyhow!("This conversation is no longer available"))?;
        if !conversation.encryption.eq_ignore_ascii_case("plain") {
            return Err(anyhow!(
                "User context requests require a plain conversation"
            ));
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
            return Err(anyhow!(
                "Choose a person who is currently in this conversation"
            ));
        }
        let now = chrono::Utc::now().timestamp_millis();
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
        request.validate(now).map_err(|error| anyhow!(error))?;
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
        .map_err(|error| ApiError::bad_request(error.to_string()))
}

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
        .map_err(|error| ApiError::bad_request(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::jwt::{BotUser, OpenIDUser};

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
