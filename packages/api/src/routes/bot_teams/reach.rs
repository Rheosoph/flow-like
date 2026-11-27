//! How a reader reaches a bot of their org, decided from facts the org view already loaded.

use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::{
    bot_team::{BotChatStatus, BotReach, CHAT_RETRY_DAYS, TalkReach, person_may_talk},
    data_classification::DataClassification,
};

/// The reader's chat row with the bot.
pub(crate) struct ChatFacts<'a> {
    pub status: BotChatStatus,
    pub conversation_id: Option<&'a str>,
    pub decided_at: Option<DateTime<FixedOffset>>,
}

/// Where the bot sits and what the reader has there.
pub(crate) struct Placement<'a> {
    pub active: bool,
    pub is_lead: bool,
    pub reader_in_team: bool,
    pub team_room: Option<&'a str>,
    pub reader_thread: Option<&'a str>,
    pub reader_level: DataClassification,
}

pub(crate) struct ReachFacts<'a> {
    pub bot_id: &'a str,
    pub owned_by_reader: bool,
    pub bot_level: Option<DataClassification>,
    pub placement: Placement<'a>,
    pub chat: Option<ChatFacts<'a>>,
}

fn reach(bot_id: &str, reach: TalkReach, conversation_id: Option<&str>) -> BotReach {
    BotReach {
        bot_id: bot_id.to_string(),
        reach,
        conversation_id: conversation_id.map(str::to_string),
        retry_at: None,
    }
}

pub(crate) fn reach_of(facts: &ReachFacts, now: DateTime<FixedOffset>) -> BotReach {
    let id = facts.bot_id;
    let place = &facts.placement;
    if facts.owned_by_reader {
        return reach(id, TalkReach::Own, None);
    }
    if !place.active {
        return reach(id, TalkReach::Unavailable, None);
    }
    if !person_may_talk(Some(place.reader_level), facts.bot_level) {
        return reach(id, TalkReach::Classified, None);
    }
    if place.reader_in_team {
        return reach(id, TalkReach::TeamRoom, place.team_room);
    }
    if place.is_lead {
        return reach(id, TalkReach::Lead, place.reader_thread);
    }
    let Some(chat) = &facts.chat else {
        return reach(id, TalkReach::Request, None);
    };
    match chat.status {
        BotChatStatus::Active => reach(id, TalkReach::Chat, chat.conversation_id),
        BotChatStatus::Pending => reach(id, TalkReach::Requested, None),
        BotChatStatus::Declined => {
            let retry_at = chat
                .decided_at
                .map(|decided| decided + Duration::days(CHAT_RETRY_DAYS))
                .filter(|retry| *retry > now);
            match retry_at {
                Some(retry) => BotReach {
                    retry_at: Some(retry.to_rfc3339()),
                    ..reach(id, TalkReach::Declined, None)
                },
                None => reach(id, TalkReach::Request, None),
            }
        }
        BotChatStatus::Ended => reach(id, TalkReach::Request, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<FixedOffset> {
        DateTime::from_timestamp(1_700_000_000, 0)
            .unwrap()
            .fixed_offset()
    }

    fn facts<'a>() -> ReachFacts<'a> {
        ReachFacts {
            bot_id: "bot",
            owned_by_reader: false,
            bot_level: None,
            placement: Placement {
                active: true,
                is_lead: false,
                reader_in_team: false,
                team_room: Some("room"),
                reader_thread: Some("thread"),
                reader_level: DataClassification::Internal,
            },
            chat: None,
        }
    }

    fn chat(status: BotChatStatus, decided_days_ago: Option<i64>) -> ChatFacts<'static> {
        ChatFacts {
            status,
            conversation_id: Some("chat-conversation"),
            decided_at: decided_days_ago.map(|days| now() - Duration::days(days)),
        }
    }

    #[test]
    fn own_wins_over_everything() {
        let mut facts = facts();
        facts.owned_by_reader = true;
        facts.placement.active = false;
        facts.bot_level = Some(DataClassification::Restricted);
        assert_eq!(reach_of(&facts, now()).reach, TalkReach::Own);
    }

    #[test]
    fn inactive_team_row_is_unavailable_even_when_classified() {
        let mut facts = facts();
        facts.placement.active = false;
        facts.bot_level = Some(DataClassification::Restricted);
        assert_eq!(reach_of(&facts, now()).reach, TalkReach::Unavailable);
    }

    #[test]
    fn a_bot_above_the_reader_is_classified_before_room_or_lead() {
        let mut facts = facts();
        facts.bot_level = Some(DataClassification::Confidential);
        facts.placement.reader_in_team = true;
        facts.placement.is_lead = true;
        assert_eq!(reach_of(&facts, now()).reach, TalkReach::Classified);
    }

    #[test]
    fn unclassified_bot_counts_as_internal() {
        let mut facts = facts();
        facts.placement.reader_level = DataClassification::Public;
        assert_eq!(reach_of(&facts, now()).reach, TalkReach::Classified);
        facts.placement.reader_level = DataClassification::Internal;
        assert_ne!(reach_of(&facts, now()).reach, TalkReach::Classified);
    }

    #[test]
    fn teammates_reach_the_team_room() {
        let mut facts = facts();
        facts.placement.reader_in_team = true;
        facts.placement.is_lead = true;
        let result = reach_of(&facts, now());
        assert_eq!(result.reach, TalkReach::TeamRoom);
        assert_eq!(result.conversation_id.as_deref(), Some("room"));
    }

    #[test]
    fn a_lead_of_another_team_is_reached_through_the_thread() {
        let mut facts = facts();
        facts.placement.is_lead = true;
        let result = reach_of(&facts, now());
        assert_eq!(result.reach, TalkReach::Lead);
        assert_eq!(result.conversation_id.as_deref(), Some("thread"));
        facts.placement.reader_thread = None;
        assert_eq!(reach_of(&facts, now()).conversation_id, None);
    }

    #[test]
    fn a_lead_ignores_a_chat_row() {
        let mut facts = facts();
        facts.placement.is_lead = true;
        facts.chat = Some(chat(BotChatStatus::Active, None));
        assert_eq!(reach_of(&facts, now()).reach, TalkReach::Lead);
    }

    #[test]
    fn an_active_chat_carries_its_conversation() {
        let mut facts = facts();
        facts.chat = Some(chat(BotChatStatus::Active, Some(1)));
        let result = reach_of(&facts, now());
        assert_eq!(result.reach, TalkReach::Chat);
        assert_eq!(result.conversation_id.as_deref(), Some("chat-conversation"));
    }

    #[test]
    fn a_pending_chat_is_requested() {
        let mut facts = facts();
        facts.chat = Some(chat(BotChatStatus::Pending, None));
        let result = reach_of(&facts, now());
        assert_eq!(result.reach, TalkReach::Requested);
        assert_eq!(result.conversation_id, None);
    }

    #[test]
    fn a_fresh_decline_waits_and_an_old_one_may_be_asked_again() {
        let mut facts = facts();
        facts.chat = Some(chat(BotChatStatus::Declined, Some(2)));
        let result = reach_of(&facts, now());
        assert_eq!(result.reach, TalkReach::Declined);
        assert_eq!(
            result.retry_at,
            Some((now() - Duration::days(2) + Duration::days(CHAT_RETRY_DAYS)).to_rfc3339())
        );
        facts.chat = Some(chat(BotChatStatus::Declined, Some(CHAT_RETRY_DAYS)));
        let result = reach_of(&facts, now());
        assert_eq!(result.reach, TalkReach::Request);
        assert_eq!(result.retry_at, None);
    }

    #[test]
    fn no_chat_row_or_an_ended_one_means_request() {
        let mut facts = facts();
        assert_eq!(reach_of(&facts, now()).reach, TalkReach::Request);
        facts.chat = Some(chat(BotChatStatus::Ended, Some(1)));
        assert_eq!(reach_of(&facts, now()).reach, TalkReach::Request);
    }
}
