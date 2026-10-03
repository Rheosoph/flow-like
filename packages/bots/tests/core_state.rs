use flow_like_bots::state::{
    BotEntry, BotsState, Day, LastRun, Outcome, Revisions, Status, WATERMARK_TTL_SECS, public_name,
    utc_date,
};
use flow_like_bots::{BotSpec, Provider};
use serde_json::{Value, json};

const LITERAL: &str = r#"{"version":1,"config_revision":12,"intent_revision":7,"decided":true,
 "bots":{"evt_helper":{"provider":"telegram","state":"connected","hold":null,"bot_id":7123456789,"bot_name":"helper_bot",
   "connected_at":1790000003,"webhook_cleared":true,"watermark":412345678,"watermark_at":1790003600,
   "last_message_at":1790003600,"last":{"at":1790003600,"finished_at":1790003612,"outcome":"succeeded"},
   "running":1,"messages":120,"runs":57,"failed":2,"ignored":61,
   "dropped_flood":0,"dropped_busy":0,"skipped_stale":3,"images_left_out":0,"reconnects":1,
   "day":{"date":"2026-10-02","runs":12}}}}"#;

fn spec(event_id: &str, event_type: &str) -> BotSpec {
    BotSpec::from_config(event_id, event_type, b"{}").unwrap()
}

#[test]
fn the_documented_literal_round_trips() {
    let literal: Value = serde_json::from_str(LITERAL).unwrap();
    let state: BotsState = serde_json::from_value(literal.clone()).unwrap();
    assert_eq!(serde_json::to_value(&state).unwrap(), literal);
    let entry = &state.bots["evt_helper"];
    assert_eq!(entry.state, Status::Connected);
    assert_eq!(entry.last.as_ref().unwrap().outcome, Outcome::Succeeded);
}

#[test]
fn a_fresh_entry_writes_every_key() {
    let state = BotsState::prepare(
        None,
        [&spec("evt_helper", "discord")],
        Revisions {
            config_revision: 3,
            intent_revision: 2,
        },
        1_790_000_000,
    );
    let written = serde_json::to_value(&state).unwrap();
    let literal: Value = serde_json::from_str(LITERAL).unwrap();
    let keys = |value: &Value| {
        let mut keys: Vec<String> = value["bots"]["evt_helper"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    assert_eq!(keys(&written), keys(&literal));
    assert_eq!(
        written["bots"]["evt_helper"],
        json!({"provider":"discord","state":"waiting","hold":null,"bot_id":null,"bot_name":null,
               "connected_at":null,"webhook_cleared":false,"watermark":null,"watermark_at":null,
               "last_message_at":null,"last":null,"running":0,"messages":0,"runs":0,"failed":0,"ignored":0,
               "dropped_flood":0,"dropped_busy":0,"skipped_stale":0,"images_left_out":0,"reconnects":0,"day":null})
    );
    assert_eq!(written["decided"], false);
    assert_eq!(written["config_revision"], 3);
}

fn stored(now: i64) -> BotsState {
    let mut entry = BotEntry::new(Provider::Telegram);
    entry.state = Status::Connected;
    entry.hold = Some("hub_unreachable".into());
    entry.bot_id = Some(7);
    entry.bot_name = Some("helper_bot".into());
    entry.connected_at = Some(now - 100);
    entry.webhook_cleared = true;
    entry.watermark = Some(500);
    entry.watermark_at = Some(now - 60);
    entry.last = Some(LastRun {
        at: now - 50,
        finished_at: now - 40,
        outcome: Outcome::Failed,
    });
    entry.running = 3;
    entry.runs = 9;
    entry.failed = 1;
    entry.reconnects = 4;
    entry.day = Some(Day {
        date: utc_date(now),
        runs: 5,
    });
    let mut state = BotsState {
        intent_revision: 7,
        config_revision: 12,
        decided: true,
        ..BotsState::default()
    };
    state.bots.insert("evt_helper".into(), entry.clone());
    state.bots.insert("evt_gone".into(), entry);
    state
}

#[test]
fn a_restart_keeps_identity_and_counters_of_one_intent() {
    let now = 1_790_035_200;
    let state = BotsState::prepare(
        Some(stored(now)),
        [&spec("evt_helper", "telegram")],
        Revisions {
            config_revision: 13,
            intent_revision: 7,
        },
        now,
    );
    assert!(!state.decided);
    assert_eq!(state.config_revision, 13);
    assert_eq!(
        state.bots.len(),
        1,
        "an event the config no longer lists is dropped"
    );
    let entry = &state.bots["evt_helper"];
    assert_eq!(entry.state, Status::Waiting);
    assert_eq!(entry.hold, None);
    assert_eq!(entry.connected_at, None);
    assert_eq!(entry.running, 0);
    assert_eq!(
        (entry.bot_id, entry.watermark, entry.webhook_cleared),
        (Some(7), Some(500), true)
    );
    assert_eq!((entry.runs, entry.failed, entry.reconnects), (9, 1, 4));
    assert_eq!(entry.day.as_ref().unwrap().runs, 5);
    assert_eq!(entry.bot_name.as_deref(), Some("helper_bot"));
}

#[test]
fn a_new_intent_starts_counters_again_but_not_the_day() {
    let now = 1_790_035_200 + 3_600;
    let state = BotsState::prepare(
        Some(stored(now)),
        [&spec("evt_helper", "telegram")],
        Revisions {
            config_revision: 14,
            intent_revision: 8,
        },
        now,
    );
    let entry = &state.bots["evt_helper"];
    assert_eq!((entry.runs, entry.failed, entry.reconnects), (0, 0, 0));
    assert!(entry.last.is_none());
    assert_eq!(entry.watermark, Some(500));
    assert_eq!(entry.day.as_ref().unwrap().runs, 5);
}

#[test]
fn what_does_not_survive() {
    let now = 1_790_035_200;
    let next_day = BotsState::prepare(
        Some(stored(now)),
        [&spec("evt_helper", "telegram")],
        Revisions {
            config_revision: 13,
            intent_revision: 7,
        },
        now + 86_400,
    );
    assert!(next_day.bots["evt_helper"].day.is_none());

    let quiet_week = BotsState::prepare(
        Some(stored(now)),
        [&spec("evt_helper", "telegram")],
        Revisions::default(),
        now + WATERMARK_TTL_SECS + 61,
    );
    let entry = &quiet_week.bots["evt_helper"];
    assert_eq!((entry.watermark, entry.watermark_at), (None, None));
    assert_eq!(entry.bot_id, Some(7));

    let other_provider = BotsState::prepare(
        Some(stored(now)),
        [&spec("evt_helper", "discord")],
        Revisions::default(),
        now,
    );
    let entry = &other_provider.bots["evt_helper"];
    assert_eq!((entry.bot_id, entry.watermark), (None, None));
    assert_eq!(entry.provider, Provider::Discord);

    let mut foreign = stored(now);
    foreign.version = 2;
    let state = BotsState::prepare(
        Some(foreign),
        [&spec("evt_helper", "telegram")],
        Revisions::default(),
        now,
    );
    assert_eq!(state.bots["evt_helper"].bot_id, None);
}

#[test]
fn another_bot_drops_the_old_bots_marks() {
    let mut entry = stored(1_790_035_200).bots.remove("evt_helper").unwrap();
    entry.adopt_bot(7);
    assert_eq!((entry.watermark, entry.webhook_cleared), (Some(500), true));
    entry.adopt_bot(8);
    assert_eq!(entry.bot_id, Some(8));
    assert_eq!(
        (
            entry.watermark,
            entry.watermark_at,
            entry.webhook_cleared,
            entry.bot_name
        ),
        (None, None, false, None)
    );
}

#[test]
fn runs_today_roll_with_the_utc_day() {
    let mut entry = BotEntry::new(Provider::Discord);
    let evening = 1_790_035_200 + 86_399;
    entry.count_run_today(evening);
    entry.count_run_today(evening);
    assert_eq!(
        entry.day,
        Some(Day {
            date: "2026-09-22".into(),
            runs: 2
        })
    );
    entry.count_run_today(evening + 1);
    assert_eq!(
        entry.day,
        Some(Day {
            date: "2026-09-23".into(),
            runs: 1
        })
    );
}

#[test]
fn dates() {
    assert_eq!(utc_date(0), "1970-01-01");
    assert_eq!(utc_date(-1), "1969-12-31");
    assert_eq!(utc_date(951_782_400), "2000-02-29");
    assert_eq!(utc_date(1_790_035_200), "2026-09-22");
    assert_eq!(utc_date(4_102_444_799), "2099-12-31");
}

#[test]
fn public_names() {
    for name in ["helper_bot", "Flow-Like Bot 2.0", "a"] {
        assert_eq!(public_name(name).as_deref(), Some(name));
    }
    for name in [
        "",
        "Grüße",
        "bot<script>",
        "emoji 🤖",
        "tab\tbot",
        &"x".repeat(65),
    ] {
        assert_eq!(public_name(name), None, "{name}");
    }
    let mut state = BotsState::default();
    let mut entry = BotEntry::new(Provider::Discord);
    entry.bot_name = Some("Grüße".into());
    state.bots.insert("evt".into(), entry);
    let state = BotsState::prepare(
        Some(state),
        [&spec("evt", "discord")],
        Revisions::default(),
        0,
    );
    assert_eq!(state.bots["evt"].bot_name, None);
}
