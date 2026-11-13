use super::{Fixture, create, fixture};
use crate::{Bot, LOCAL_OWNER_ID, TaskAuthority, ToolTurn};
use flow_like_types::{
    bot_task::{CreateBotTask, TaskCreator, TaskKind, UpdateBotTask},
    bot_turn::turn_id,
    inbox::is_tombstone,
};
use serde_json::json;

const ASKED: &str = "Every morning at 8, mail me the sales summary";

fn scheduler_bot(f: &Fixture) -> Bot {
    f.store
        .create_bot(create(json!({
            "name": "Clock",
            "appearance": { "kind": "pip" },
            "capabilities": ["scheduling"],
        })))
        .unwrap()
}

fn daily(instruction: &str) -> CreateBotTask {
    CreateBotTask {
        kind: Some(TaskKind::Cron),
        cron: Some("0 8 * * *".into()),
        tz: Some("Europe/Berlin".into()),
        instruction: instruction.into(),
        ..Default::default()
    }
}

fn say(f: &Fixture, conversation_id: &str, sender: &str, message_id: &str, text: &str) {
    f.send(
        conversation_id,
        sender,
        message_id,
        "message",
        json!({ "text": text }),
    )
    .unwrap();
}

fn authority(f: &Fixture, bot: &Bot, task_id: &str) -> TaskAuthority {
    f.store.task_authority(&bot.id, task_id).unwrap().unwrap()
}

/// A turn in `conversation_id`; an empty `turn` is one whose id the store cannot match.
fn in_turn<'a>(conversation_id: &'a str, turn: &'a str) -> ToolTurn<'a> {
    ToolTurn {
        conversation_id: Some(conversation_id),
        turn_id: Some(turn).filter(|turn| !turn.is_empty()),
    }
}

/// Whether the person wrote the task, and the message that asked for it.
fn proof(f: &Fixture, bot: &Bot, task_id: &str) -> (bool, Option<String>) {
    let authority = authority(f, bot, task_id);
    (
        authority.owner_written,
        authority.request.map(|request| request.message_id),
    )
}

#[test]
fn a_task_keeps_the_message_that_asked_for_it_with_its_edits_and_deletion() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let chat = f.direct(&bot);
    say(&f, &chat.id, LOCAL_OWNER_ID, "ask", ASKED);
    let asking = turn_id(&bot.id, "ask");
    let task = f
        .store
        .create_bot_task(
            &bot.id,
            daily("Mail the sales summary"),
            in_turn(&chat.id, &asking),
        )
        .unwrap();
    assert_eq!(task.created_by, TaskCreator::Bot);
    assert_eq!(task.conversation, None);
    let read = authority(&f, &bot, &task.id);
    assert!(!read.owner_written);
    let request = read.request.expect("the person's message is kept");
    assert_eq!(request.conversation_id, chat.id);
    assert_eq!(request.message_id, "ask");
    assert_eq!(request.entries.len(), 1);
    assert_eq!(request.entries[0].envelope["body"]["text"], ASKED);

    f.send(
        &chat.id,
        LOCAL_OWNER_ID,
        "ask_edit",
        "edit",
        json!({"target": "ask", "text": "Every morning at 8, mail me the totals"}),
    )
    .unwrap();
    say(&f, &chat.id, LOCAL_OWNER_ID, "unrelated", "Thanks");
    let entries = authority(&f, &bot, &task.id).request.unwrap().entries;
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.message_id.as_str())
            .collect::<Vec<_>>(),
        ["ask", "ask_edit"]
    );

    f.send(
        &chat.id,
        LOCAL_OWNER_ID,
        "ask_delete",
        "delete",
        json!({"target": "ask"}),
    )
    .unwrap();
    let entries = authority(&f, &bot, &task.id).request.unwrap().entries;
    assert!(is_tombstone(&entries[0].envelope), "{entries:?}");

    f.store.delete_task(&bot.id, &task.id).unwrap();
    assert_eq!(f.store.task_authority(&bot.id, &task.id).unwrap(), None);
}

#[test]
fn only_the_persons_own_message_in_their_chat_with_the_bot_counts() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let other = f.bot("Other");
    let chat = f.direct(&bot);
    let group = f.group(&[&bot, &other]);
    say(&f, &group.id, LOCAL_OWNER_ID, "group_ask", ASKED);
    say(&f, &chat.id, &bot.id, "bot_says", ASKED);
    say(&f, &chat.id, LOCAL_OWNER_ID, "ask", ASKED);
    let owned = f
        .store
        .create_task(&bot.id, daily("Draft the report"), TaskCreator::Owner)
        .unwrap();
    assert_eq!(proof(&f, &bot, &owned.id), (true, None));
    let due = chrono::DateTime::parse_from_rfc3339(owned.next_run_at.as_deref().unwrap())
        .unwrap()
        .timestamp_micros();
    f.store.fire_due_tasks(due + 1).unwrap();
    let trigger = f
        .all_entries(&chat.id)
        .into_iter()
        .find(|entry| entry.kind == flow_like_types::inbox::InboxEntryKind::Trigger)
        .expect("the run's trigger")
        .message_id;

    let cases = [
        (Some(group.id.as_str()), turn_id(&bot.id, "group_ask")),
        (Some(chat.id.as_str()), turn_id(&bot.id, "bot_says")),
        (Some(chat.id.as_str()), turn_id(&bot.id, &trigger)),
        (Some(chat.id.as_str()), turn_id(&other.id, "ask")),
        (Some(group.id.as_str()), turn_id(&bot.id, "ask")),
        (None, turn_id(&bot.id, "ask")),
        (Some(chat.id.as_str()), "bt_unknown".to_string()),
    ];
    for (conversation_id, turn) in cases {
        let task = f
            .store
            .create_bot_task(
                &bot.id,
                daily("Mail the summary"),
                ToolTurn {
                    conversation_id,
                    turn_id: Some(turn.as_str()),
                },
            )
            .unwrap();
        assert_eq!(proof(&f, &bot, &task.id), (false, None), "{turn}");
        f.store.delete_task(&bot.id, &task.id).unwrap();
    }
    let legacy = f
        .store
        .create_task_in(&bot.id, daily("Mail it"), TaskCreator::Bot, Some(&chat.id))
        .unwrap();
    assert_eq!(proof(&f, &bot, &legacy.id), (false, None));
}

#[test]
fn a_rewrite_moves_the_authority_to_its_author_and_an_unprompted_one_drops_it() {
    let f = fixture();
    let bot = scheduler_bot(&f);
    let chat = f.direct(&bot);
    say(&f, &chat.id, LOCAL_OWNER_ID, "ask", ASKED);
    say(
        &f,
        &chat.id,
        LOCAL_OWNER_ID,
        "again",
        "Make it the weekly summary",
    );
    let (asking, again) = (turn_id(&bot.id, "ask"), turn_id(&bot.id, "again"));
    let task = f
        .store
        .create_bot_task(
            &bot.id,
            daily("Mail the summary"),
            in_turn(&chat.id, &asking),
        )
        .unwrap();
    let instruction = |text: &str| UpdateBotTask {
        instruction: Some(text.into()),
        ..Default::default()
    };

    f.store
        .update_bot_task(
            &bot.id,
            &task.id,
            UpdateBotTask {
                title: Some("Sales".into()),
                cron: Some("0 9 * * *".into()),
                instruction: Some("Mail the summary".into()),
                ..Default::default()
            },
            in_turn(&chat.id, ""),
        )
        .unwrap();
    assert_eq!(proof(&f, &bot, &task.id), (false, Some("ask".into())));

    let weekly = instruction("Mail the weekly");
    f.store
        .update_bot_task(&bot.id, &task.id, weekly, in_turn(&chat.id, &again))
        .unwrap();
    assert_eq!(proof(&f, &bot, &task.id), (false, Some("again".into())));

    let everyone = instruction("Mail everyone");
    f.store
        .update_bot_task(&bot.id, &task.id, everyone, in_turn(&chat.id, ""))
        .unwrap();
    assert_eq!(proof(&f, &bot, &task.id), (false, None));

    f.store
        .update_task(&bot.id, &task.id, instruction("Mail me the summary"))
        .unwrap();
    assert_eq!(proof(&f, &bot, &task.id), (true, None));

    f.store
        .update_task_in_turn(
            &bot.id,
            &task.id,
            instruction("Mail the team"),
            Some(&chat.id),
        )
        .unwrap();
    assert_eq!(proof(&f, &bot, &task.id), (false, None));
}
