//! Context is supplied by the consenting conversation participant's client, wherever the bot runs.

use super::schedule::TaskApi;
use crate::functions::local_bots::{LocalBots, on_store};
use async_trait::async_trait;
use flow_like_bot_tools::user_context::{
    UserContextAvailability, UserContextBackend, UserContextRequestArgs, UserContextRequestReceipt,
};
use flow_like_types::{
    Result, anyhow,
    bot::BotCapability,
    bot_user_context::{UserContextAudience, UserContextField, UserContextRequest},
    inbox::{ConversationKind, InboxEntryKind, PrincipalKind, plain_envelope},
};
use reqwest::Method;
use serde_json::json;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

struct HubContext {
    api: Arc<dyn TaskApi>,
    bot_id: String,
    conversation_id: String,
}

impl HubContext {
    fn path(&self, action: &str) -> String {
        format!(
            "bots/{}/user-context/{action}",
            super::super::native_platform_tools::segment(&self.bot_id)
        )
    }
}

#[async_trait]
impl UserContextBackend for HubContext {
    async fn available(&self) -> Result<UserContextAvailability> {
        let path = format!(
            "{}?conversation_id={}",
            self.path("available"),
            urlencoding::encode(&self.conversation_id)
        );
        let value = self
            .api
            .request(Method::GET, &path, None)
            .await
            .map_err(|error| anyhow!(error))?;
        Ok(serde_json::from_value(value)?)
    }

    async fn request(&self, args: UserContextRequestArgs) -> Result<UserContextRequestReceipt> {
        let mut body = serde_json::to_value(args)?;
        body["conversation_id"] = json!(self.conversation_id);
        let value = self
            .api
            .request(Method::POST, &self.path("request"), Some(&body))
            .await
            .map_err(|error| anyhow!(error))?;
        Ok(serde_json::from_value(value)?)
    }
}

struct LocalContext {
    app: AppHandle,
    bot_id: String,
    conversation_id: String,
}

impl LocalContext {
    async fn conversation(&self) -> Result<flow_like_local_bots::Conversation> {
        let bots = self
            .app
            .try_state::<LocalBots>()
            .ok_or_else(|| anyhow!("Local bots are unavailable"))?;
        let (bot, room) = (self.bot_id.clone(), self.conversation_id.clone());
        let (bot, conversation) = on_store(&bots, move |store| {
            Ok((store.get_bot(&bot)?, store.conversation_as(&room, &bot)?))
        })
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
        if !bot.is_some_and(|bot| bot.capabilities.contains(&BotCapability::UserContext)) {
            return Err(anyhow!("The bot's User context capability is off"));
        }
        conversation.ok_or_else(|| anyhow!("The bot is no longer in this conversation"))
    }
}

#[async_trait]
impl UserContextBackend for LocalContext {
    async fn available(&self) -> Result<UserContextAvailability> {
        let conversation = self.conversation().await?;
        Ok(serde_json::from_value(json!({
            "participants": conversation.members.iter().filter(|member| member.kind == PrincipalKind::User)
                .map(|member| json!({"id": member.principal_id, "name": "You"})).collect::<Vec<_>>(),
            "fields": UserContextField::ALL,
            "device_support": "unknown_until_shared",
        }))?)
    }

    async fn request(&self, args: UserContextRequestArgs) -> Result<UserContextRequestReceipt> {
        let conversation = self.conversation().await?;
        if !conversation.members.iter().any(|member| {
            member.kind == PrincipalKind::User && member.principal_id == args.subject_id
        }) {
            return Err(anyhow!(
                "Choose a person who is currently in this conversation"
            ));
        }
        let request = UserContextRequest::new(
            flow_like_types::create_id(),
            self.bot_id.clone(),
            conversation.id.clone(),
            args,
            match conversation.kind {
                ConversationKind::Direct => UserContextAudience::Direct,
                ConversationKind::Group => UserContextAudience::Group,
            },
            conversation
                .members
                .iter()
                .map(|member| member.principal_id.clone())
                .collect(),
            chrono::Utc::now().timestamp_millis(),
        );
        request
            .validate(chrono::Utc::now().timestamp_millis())
            .map_err(|error| anyhow!(error))?;
        let receipt = UserContextRequestReceipt {
            request_id: request.id.clone(),
            conversation_id: request.conversation_id.clone(),
            subject_id: request.subject_id.clone(),
            expires_at: request.expires_at,
        };
        let outgoing = flow_like_local_bots::SendEntryRequest {
            message_id: request.id.clone(),
            kind: InboxEntryKind::Message,
            envelope: Some(plain_envelope(json!({
                "text": "Please choose whether to share the requested context.",
                "user_context_request": request,
            }))),
            envelopes: None,
        };
        let bots = self
            .app
            .try_state::<LocalBots>()
            .ok_or_else(|| anyhow!("Local bots are unavailable"))?;
        let (bot, room) = (self.bot_id.clone(), self.conversation_id.clone());
        on_store(&bots, move |store| store.send(&room, &bot, outgoing))
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(receipt)
    }
}

pub(crate) async fn context_for(
    app: &AppHandle,
    bot_id: &str,
    conversation_id: Option<&str>,
    api: Arc<dyn TaskApi>,
) -> Option<Arc<dyn UserContextBackend>> {
    let conversation_id = conversation_id?.to_owned();
    if super::held_locally(app, bot_id).await {
        Some(Arc::new(LocalContext {
            app: app.clone(),
            bot_id: bot_id.to_owned(),
            conversation_id,
        }))
    } else {
        Some(Arc::new(HubContext {
            api,
            bot_id: bot_id.to_owned(),
            conversation_id,
        }))
    }
}
