//! What a team change owes the world once its transaction committed: people whose inbox changed
//! and people told about an invitation.

use crate::{
    entity::sea_orm_active_enums::NotificationType,
    push_notifications::{DispatchNotificationInput, dispatch_notification},
    routes::inbox::{invitations::person_name, wake},
    state::AppState,
};

/// An invitation notice: `{inviter} {subject}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub user_id: String,
    pub inviter: String,
    pub subject: String,
}

#[derive(Debug, Default)]
pub(crate) struct Effects {
    pub wake: Vec<String>,
    pub notices: Vec<Notice>,
}

impl Effects {
    pub fn wake(&mut self, people: Vec<String>) {
        self.wake.extend(people);
    }

    pub fn notice(&mut self, user_id: &str, inviter: &str, subject: String) {
        self.notices.push(Notice {
            user_id: user_id.to_string(),
            inviter: inviter.to_string(),
            subject,
        });
    }
}

/// Wakes the touched people and sends the notices; a failed notice is logged, the invitation
/// stands without it.
pub(crate) async fn settle(state: &AppState, mut effects: Effects) {
    effects.wake.sort();
    effects.wake.dedup();
    wake(state, effects.wake).await;
    for notice in effects.notices {
        let who = person_name(state, &notice.inviter).await;
        let result = dispatch_notification(
            state,
            DispatchNotificationInput {
                notification_priority: Default::default(),
                bot_context: None,
                user_id: notice.user_id.clone(),
                app_id: None,
                title: format!("{who} {}", notice.subject),
                description: Some("Accept or decline in your bot teams.".to_string()),
                icon: Some("users".to_string()),
                image: None,
                link: Some("/bots".to_string()),
                notification_type: NotificationType::System,
                source_run_id: None,
                source_node_id: None,
            },
        )
        .await;
        if let Err(error) = result {
            tracing::warn!(%error, user = %notice.user_id, "Failed to dispatch a bot team invitation notice");
        }
    }
}
