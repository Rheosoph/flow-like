//! The emails of a bot's room invitation: to an address without an account, with the link that
//! signs them up and joins them; and to a person away for a week, beside the push. Both name the
//! bot and its owner, the room and the reason the owner approved. Addresses never enter logs.

use chrono::{DateTime, Duration, FixedOffset, Utc};
use sea_orm::{EntityTrait, QuerySelect};

use crate::{
    entity::{bot, conversation, conversation_invite_link, user},
    error::ApiError,
    mail::{
        DynMailClient, EmailMessage,
        layout::one_line,
        templates::{RoomInvitationMail, room_invitation},
    },
    routes::{inbox::invitations::person_name, invite},
    state::AppState,
};

/// Days without a signed-in app after which an invitation also goes out by email.
pub(crate) const AWAY_DAYS: i64 = 7;
const SUBJECT_MAX_CHARS: usize = 150;
const ROOM_MAX_CHARS: usize = 80;

/// Whether someone last seen at `last_seen` counts as away. Never seen counts as away too: the
/// column is not backfilled, so for a week after it ships that includes people who have not
/// opened the app since, and they get an email a few days early.
pub(crate) fn away(last_seen: Option<DateTime<FixedOffset>>, now: DateTime<FixedOffset>) -> bool {
    last_seen.is_none_or(|seen| seen <= now - Duration::days(AWAY_DAYS))
}

/// The inviting bot, its owner and the room, as the email names them.
struct Names {
    bot: String,
    owner: String,
    room: Option<String>,
}

impl Names {
    fn subject(&self) -> String {
        let room = self
            .room
            .as_deref()
            .map_or_else(|| "a group chat".to_string(), |room| format!("“{room}”"));
        one_line(
            &format!("{} ({}’s bot) invited you to {room}", self.bot, self.owner),
            SUBJECT_MAX_CHARS,
        )
    }
}

async fn names(state: &AppState, bot_id: &str, conversation_id: &str) -> Result<Names, ApiError> {
    let bot = bot::Entity::find_by_id(bot_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Bot '{bot_id}' does not exist")))?;
    let room = conversation::Entity::find_by_id(conversation_id)
        .select_only()
        .column(conversation::Column::Title)
        .into_tuple::<Option<String>>()
        .one(&state.db)
        .await?
        .flatten()
        .map(|title| one_line(&title, ROOM_MAX_CHARS))
        .filter(|title| !title.is_empty());
    Ok(Names {
        bot: one_line(&crate::bot_notify::display_name(&bot), ROOM_MAX_CHARS),
        owner: one_line(&person_name(state, &bot.owner_id).await, ROOM_MAX_CHARS),
        room,
    })
}

async fn send(
    mail: &DynMailClient,
    to: &str,
    names: &Names,
    invitation: RoomInvitationMail<'_>,
) -> Result<(), ApiError> {
    let (html, text) = room_invitation(&invitation);
    mail.send(EmailMessage {
        to: to.to_string(),
        subject: names.subject(),
        body_html: Some(html),
        body_text: Some(text),
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "The email of the room invitation could not be sent: {error}"
        ))
    })
}

/// Emails the single-use `link` of a bot's invitation to the address it was made for.
pub(crate) async fn send_to_newcomer(
    state: &AppState,
    mail: &DynMailClient,
    link: &conversation_invite_link::Model,
) -> Result<(), ApiError> {
    let address = link.email.as_deref().ok_or_else(|| {
        ApiError::internal(format!(
            "Invite link '{}' is not an email invitation",
            link.id
        ))
    })?;
    let names = names(state, &link.created_by, &link.conversation_id).await?;
    let url = invite::room_link_url(state, &link.token)?;
    send(
        mail,
        address,
        &names,
        RoomInvitationMail {
            bot: &names.bot,
            owner: &names.owner,
            room: names.room.as_deref(),
            reason: link.reason.as_deref(),
            url: &url,
            link_expires: link.expires_at,
        },
    )
    .await
}

/// Emails a pending invitation to `person_id` when they have not opened Flow-Like for
/// [`AWAY_DAYS`] days. Failures are logged: the invitation and its push stand without it.
pub(crate) async fn email_if_away(
    state: &AppState,
    bot_id: &str,
    conversation_id: &str,
    person_id: &str,
    reason: Option<&str>,
) {
    let Some(mail) = state.mail_client.clone() else {
        return;
    };
    let sent = async {
        let Some((Some(address), last_seen)) = user::Entity::find_by_id(person_id)
            .select_only()
            .column(user::Column::Email)
            .column(user::Column::LastSeenAt)
            .into_tuple::<(Option<String>, Option<DateTime<FixedOffset>>)>()
            .one(&state.db)
            .await?
        else {
            return Ok(false);
        };
        if !away(last_seen, Utc::now().fixed_offset()) {
            return Ok(false);
        }
        let names = names(state, bot_id, conversation_id).await?;
        let url = invite::web_invitations_url(state)?;
        send(
            &mail,
            &address,
            &names,
            RoomInvitationMail {
                bot: &names.bot,
                owner: &names.owner,
                room: names.room.as_deref(),
                reason,
                url: &url,
                link_expires: None,
            },
        )
        .await
        .map(|()| true)
    };
    match sent.await {
        Ok(true) => {
            tracing::info!(
                conversation_id,
                bot_id,
                "Emailed a room invitation to a person away"
            )
        }
        Ok(false) => {}
        Err(error) => tracing::warn!(
            %error,
            conversation_id,
            bot_id,
            "Failed to email a room invitation to a person away"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(days_ago: i64) -> Option<DateTime<FixedOffset>> {
        Some(Utc::now().fixed_offset() - Duration::days(days_ago) - Duration::minutes(1))
    }

    #[test]
    fn a_week_without_the_app_or_never_seen_counts_as_away() {
        let now = Utc::now().fixed_offset();
        assert!(!away(at(0), now));
        assert!(!away(at(6), now));
        assert!(away(at(7), now));
        assert!(away(at(30), now));
        assert!(away(None, now));
    }

    #[test]
    fn the_subject_names_the_bot_its_owner_and_the_room_on_one_line() {
        let named = Names {
            bot: "Flux".into(),
            owner: "Felix".into(),
            room: Some("Launch".into()),
        };
        assert_eq!(
            named.subject(),
            "Flux (Felix’s bot) invited you to “Launch”"
        );
        let untitled = Names {
            room: None,
            ..named
        };
        assert_eq!(
            untitled.subject(),
            "Flux (Felix’s bot) invited you to a group chat"
        );
    }
}
