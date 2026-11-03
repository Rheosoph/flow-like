//! The server's `MailBackend`: a bot's mail through the platform mail client, from the bot's own
//! address, recorded in the bot's mail threads.

use crate::state::AppState;
use async_trait::async_trait;
use flow_like_bot_tools::email::{MailBackend, MailFailure, OutgoingMail, ReplyMail, SentMail};

pub(crate) struct ApiMail {
    state: AppState,
    bot_id: String,
}

impl ApiMail {
    pub(crate) fn new(state: &AppState, bot_id: &str) -> Self {
        Self {
            state: state.clone(),
            bot_id: bot_id.to_string(),
        }
    }
}

#[async_trait]
impl MailBackend for ApiMail {
    async fn send(&self, mail: OutgoingMail) -> Result<SentMail, MailFailure> {
        crate::bot_mail::send(&self.state, &self.bot_id, mail).await
    }

    async fn reply(&self, mail: ReplyMail) -> Result<SentMail, MailFailure> {
        crate::bot_mail::reply(&self.state, &self.bot_id, mail).await
    }
}
