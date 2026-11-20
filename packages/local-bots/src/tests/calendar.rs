use super::{Fixture, create, fixture};
use crate::{Bot, LOCAL_OWNER_ID};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use flow_like_types::{
    bot_calendar::{
        EventWake, ExternalEventRef, MAX_EVENTS_PER_BOT, TaskEvent, UpdateCalendarWake,
        WAKE_NOTE_PREFIX,
    },
    bot_task::{BotTask, CreateBotTask, TaskCreator, TaskKind, UpdateBotTask},
    inbox::InboxEntryKind,
};
use serde_json::json;

fn calendar_bot(f: &Fixture) -> Bot {
    f.store
        .create_bot(create(json!({
            "name": "Secretary",
            "appearance": { "kind": "pip" },
            "capabilities": ["calendar", "scheduling"],
        })))
        .unwrap()
}

fn in_hours(hours: i64) -> DateTime<Utc> {
    DateTime::from_timestamp((Utc::now() + Duration::hours(hours)).timestamp(), 0).unwrap()
}

fn text(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn event_at(starts_at: DateTime<Utc>) -> TaskEvent {
    TaskEvent {
        starts_at: text(starts_at),
        ends_at: None,
        all_day: false,
        location: None,
        wake: None,
        wake_offsets: Vec::new(),
        external: None,
        link: None,
    }
}

fn event_task(title: &str, starts_at: DateTime<Utc>) -> CreateBotTask {
    CreateBotTask {
        kind: Some(TaskKind::Event),
        title: Some(title.into()),
        instruction: "Bring the slides".into(),
        event: Some(event_at(starts_at)),
        ..Default::default()
    }
}

fn external_ref(id: &str) -> ExternalEventRef {
    ExternalEventRef {
        connector_id: "conn1".into(),
        provider: "google".into(),
        calendar_id: "primary".into(),
        event_id: id.into(),
    }
}

fn wakes_on(f: &Fixture, bot: &Bot) {
    f.store
        .set_calendar_wake(
            &bot.id,
            UpdateCalendarWake {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
}

fn next(task: &BotTask) -> Option<DateTime<Utc>> {
    task.next_run_at.as_deref().map(|at| {
        DateTime::parse_from_rfc3339(at)
            .unwrap()
            .with_timezone(&Utc)
    })
}

fn start_of(task: &BotTask) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&task.event.as_ref().unwrap().starts_at)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn an_event_needs_the_calendar_capability_and_the_owner_chat() {
    let f = fixture();
    let plain = f.bot("Plain");
    assert!(
        f.store
            .create_task(
                &plain.id,
                event_task("Dentist", in_hours(5)),
                TaskCreator::Owner
            )
            .is_err()
    );
    let scheduling_only = f
        .store
        .create_bot(create(json!({
            "name": "Clock",
            "appearance": { "kind": "pip" },
            "capabilities": ["scheduling"],
        })))
        .unwrap();
    assert!(
        f.store
            .create_task(
                &scheduling_only.id,
                event_task("Dentist", in_hours(5)),
                TaskCreator::Owner
            )
            .is_err()
    );

    let bot = calendar_bot(&f);
    let group = f.group(&[&bot]);
    let refused = f.store.create_task_in(
        &bot.id,
        event_task("Dentist", in_hours(5)),
        TaskCreator::Bot,
        Some(&group.id),
    );
    assert!(refused.unwrap_err().message().contains("own chat"));

    let event = f
        .store
        .create_task(
            &bot.id,
            event_task("Dentist", in_hours(5)),
            TaskCreator::Owner,
        )
        .unwrap();
    assert_eq!(event.kind, TaskKind::Event);
    assert!(event.conversation.is_none());
    for bad in [
        CreateBotTask {
            at: Some(text(in_hours(3))),
            ..event_task("Dentist", in_hours(5))
        },
        CreateBotTask {
            event: None,
            ..event_task("Dentist", in_hours(5))
        },
        CreateBotTask {
            title: None,
            ..event_task("Dentist", in_hours(5))
        },
        CreateBotTask {
            cron: Some("* * * * *".into()),
            ..event_task("Dentist", in_hours(5))
        },
    ] {
        assert!(
            f.store
                .create_task(&bot.id, bad, TaskCreator::Owner)
                .is_err()
        );
    }
}

#[test]
fn events_have_their_own_cap_and_do_not_use_up_the_task_cap() {
    let f = fixture();
    let bot = calendar_bot(&f);
    for _ in 0..flow_like_types::bot_task::MAX_TASKS_PER_BOT {
        f.store
            .create_task(
                &bot.id,
                CreateBotTask {
                    kind: Some(TaskKind::Once),
                    at: Some(text(in_hours(2))),
                    instruction: "Check".into(),
                    ..Default::default()
                },
                TaskCreator::Owner,
            )
            .unwrap();
    }
    for index in 0..MAX_EVENTS_PER_BOT {
        f.store
            .create_task(
                &bot.id,
                event_task(&format!("Event {index}"), in_hours(5)),
                TaskCreator::Owner,
            )
            .unwrap();
    }
    assert!(
        f.store
            .create_task(
                &bot.id,
                event_task("One more", in_hours(5)),
                TaskCreator::Owner
            )
            .is_err()
    );
}

#[test]
fn an_event_wakes_only_as_the_bot_settings_and_its_own_wake_say() {
    let f = fixture();
    let bot = calendar_bot(&f);
    let start = in_hours(5);
    let silent = f
        .store
        .create_task(&bot.id, event_task("Dentist", start), TaskCreator::Owner)
        .unwrap();
    assert!(!silent.enabled && silent.next_run_at.is_none());
    assert!(silent.event.as_ref().unwrap().wake_offsets.is_empty());

    let own = f
        .store
        .create_task(
            &bot.id,
            CreateBotTask {
                event: Some(TaskEvent {
                    wake: Some(EventWake {
                        enabled: true,
                        offsets_minutes: vec![30],
                    }),
                    ..event_at(start)
                }),
                ..event_task("Standup", start)
            },
            TaskCreator::Owner,
        )
        .unwrap();
    assert_eq!(next(&own).unwrap(), start - Duration::minutes(30));
    assert_eq!(own.event.as_ref().unwrap().wake_offsets, vec![30]);

    wakes_on(&f, &bot);
    let tasks = f.store.tasks(&bot.id).unwrap();
    let find = |title: &str| tasks.iter().find(|task| task.title == title).unwrap();
    assert_eq!(
        next(find("Dentist")).unwrap(),
        start - Duration::minutes(15)
    );
    assert!(find("Dentist").enabled && find("Dentist").monitoring);
    assert_eq!(
        next(find("Standup")).unwrap(),
        start - Duration::minutes(30)
    );

    f.store
        .set_calendar_wake(
            &bot.id,
            UpdateCalendarWake {
                skip_matching: Some(vec!["dent".into()]),
                quiet: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
    let tasks = f.store.tasks(&bot.id).unwrap();
    let dentist = tasks.iter().find(|task| task.title == "Dentist").unwrap();
    assert!(!dentist.enabled && dentist.next_run_at.is_none());
    let standup = tasks.iter().find(|task| task.title == "Standup").unwrap();
    assert!(standup.enabled && !standup.monitoring);
    assert_eq!(
        f.store.calendar_wake(&bot.id).unwrap().skip_matching,
        vec!["dent"]
    );
}

#[test]
fn an_external_event_is_upserted_and_always_bot_authored() {
    let f = fixture();
    let bot = calendar_bot(&f);
    wakes_on(&f, &bot);
    f.store
        .set_calendar_wake(
            &bot.id,
            UpdateCalendarWake {
                external: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    let request = |title: &str, starts_at| CreateBotTask {
        event: Some(TaskEvent {
            external: Some(external_ref("abc")),
            ..event_at(starts_at)
        }),
        ..event_task(title, starts_at)
    };
    let created = f
        .store
        .create_task(&bot.id, request("Invite", in_hours(6)), TaskCreator::Owner)
        .unwrap();
    let authority = f
        .store
        .task_authority(&bot.id, &created.id)
        .unwrap()
        .unwrap();
    assert!(!authority.owner_written && authority.request.is_none());

    let moved = f
        .store
        .create_task(
            &bot.id,
            request("Invite moved", in_hours(8)),
            TaskCreator::Bot,
        )
        .unwrap();
    assert_eq!(moved.id, created.id);
    assert_eq!(moved.title, "Invite moved");
    assert_eq!(
        next(&moved).unwrap(),
        start_of(&moved) - Duration::minutes(15)
    );
    assert_eq!(f.store.tasks(&bot.id).unwrap().len(), 1);

    let other = f
        .store
        .create_task(
            &bot.id,
            CreateBotTask {
                event: Some(TaskEvent {
                    external: Some(external_ref("def")),
                    ..event_at(in_hours(6))
                }),
                ..event_task("Another", in_hours(6))
            },
            TaskCreator::Bot,
        )
        .unwrap();
    assert_ne!(other.id, created.id);
}

#[test]
fn updating_an_event_moves_its_wake_up() {
    let f = fixture();
    let bot = calendar_bot(&f);
    wakes_on(&f, &bot);
    let start = in_hours(5);
    let task = f
        .store
        .create_task(&bot.id, event_task("Review", start), TaskCreator::Owner)
        .unwrap();
    let later = in_hours(9);
    let moved = f
        .store
        .update_task(
            &bot.id,
            &task.id,
            UpdateBotTask {
                event: Some(event_at(later)),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(next(&moved).unwrap(), later - Duration::minutes(15));
    assert!(moved.last_run_at.is_none());

    let silenced = f
        .store
        .update_task(
            &bot.id,
            &task.id,
            UpdateBotTask {
                event: Some(TaskEvent {
                    wake: Some(EventWake {
                        enabled: false,
                        offsets_minutes: Vec::new(),
                    }),
                    ..event_at(later)
                }),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!silenced.enabled && silenced.next_run_at.is_none());

    let placed = f.store.update_task(
        &bot.id,
        &task.id,
        UpdateBotTask {
            at: Some(text(later)),
            ..Default::default()
        },
    );
    assert!(placed.is_err());
}

#[test]
fn a_due_wake_up_fires_in_the_owner_chat_with_the_wake_text() {
    let f = fixture();
    let bot = calendar_bot(&f);
    wakes_on(&f, &bot);
    let start = in_hours(1);
    let task = f
        .store
        .create_task(&bot.id, event_task("Review", start), TaskCreator::Owner)
        .unwrap();
    let due = next(&task).unwrap();
    assert!(
        f.store
            .fire_due_tasks((due - Duration::seconds(5)).timestamp_micros())
            .unwrap()
            .is_empty()
    );
    let fired = f
        .store
        .fire_due_tasks((due + Duration::seconds(1)).timestamp_micros())
        .unwrap();
    assert_eq!(fired.len(), 1);
    assert!(!fired[0].enabled && fired[0].next_run_at.is_none());

    let conversation = f.direct(&bot);
    let triggers: Vec<_> = f
        .all_entries(&conversation.id)
        .into_iter()
        .filter(|entry| entry.kind == InboxEntryKind::Trigger)
        .collect();
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].sender_id, LOCAL_OWNER_ID);
    let body = &triggers[0].envelope["body"];
    assert_eq!(body["monitoring"], true);
    assert_eq!(body["ref_id"], task.id);
    assert!(body["text"].as_str().unwrap().starts_with(WAKE_NOTE_PREFIX));
    assert!(body["text"].as_str().unwrap().contains(&task.id));
}

#[test]
fn a_repeating_event_wakes_again_after_it_fired() {
    let f = fixture();
    let bot = calendar_bot(&f);
    wakes_on(&f, &bot);
    let task = f
        .store
        .create_task(
            &bot.id,
            CreateBotTask {
                cron: Some("0 9 * * *".into()),
                tz: Some("UTC".into()),
                ..event_task("Daily", in_hours(1))
            },
            TaskCreator::Owner,
        )
        .unwrap();
    assert_eq!(task.cron.as_deref(), Some("0 9 * * *"));
    let due = next(&task).unwrap();
    let fired = f
        .store
        .fire_due_tasks((due + Duration::seconds(1)).timestamp_micros())
        .unwrap();
    assert_eq!(fired.len(), 1);
    assert!(fired[0].enabled);
    assert!(next(&fired[0]).unwrap() > due);
}
