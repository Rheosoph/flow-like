use super::{
    permissions_tests::database,
    presence::{
        BotPresence, ONLINE_WINDOW_SECS, PresenceState, forget_in, presence_in, presence_of,
        report_in,
    },
};
use crate::routes::inbox::fixture::{at, delete_bot, insert_bot, insert_flux};
use chrono::Duration;
use flow_like_types::{bot::BotLocation, bot_turn::TurnHostKind};
use sea_orm::DatabaseConnection;

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn state(presence: &std::collections::BTreeMap<String, BotPresence>, bot: &str) -> PresenceState {
    presence[bot].state
}

async fn bots(db: &DatabaseConnection) {
    insert_bot(db, "bot_server", "user-a", "SERVER").await;
    insert_bot(db, "bot_desktop", "user-a", "DESKTOP").await;
    insert_bot(db, "bot_device", "user-a", "DEVICE").await;
    insert_bot(db, "bot_theirs", "user-b", "DESKTOP").await;
}

#[test]
fn the_online_window_ends_after_150_seconds() {
    let now = at(10_000);
    let seen = |seconds: i64| {
        presence_of(
            BotLocation::Desktop,
            Some(now - Duration::seconds(seconds)),
            now,
        )
        .state
    };
    assert_eq!(ONLINE_WINDOW_SECS, 150);
    assert_eq!(seen(0), PresenceState::Online);
    assert_eq!(seen(149), PresenceState::Online);
    assert_eq!(seen(150), PresenceState::Online);
    assert_eq!(seen(151), PresenceState::Offline);
    assert_eq!(seen(86_400), PresenceState::Offline);
    assert_eq!(
        presence_of(BotLocation::Device, None, now).state,
        PresenceState::Unknown
    );
}

#[test]
fn server_bots_are_online_whatever_the_heartbeats_say() {
    let now = at(10_000);
    for seen in [None, Some(now - Duration::days(30))] {
        let presence = presence_of(BotLocation::Server, seen, now);
        assert_eq!(presence.state, PresenceState::Online);
        assert_eq!(presence.last_seen_at, None);
    }
}

#[tokio::test]
async fn one_query_reports_every_live_bot_of_the_caller() {
    let db = database().await;
    bots(&db).await;
    insert_bot(&db, "bot_gone", "user-a", "DESKTOP").await;
    delete_bot(&db, "bot_gone").await;
    insert_bot(&db, "bot_stale", "user-a", "DESKTOP").await;
    let now = at(10_000);
    report_in(
        &db,
        "user-a",
        "desk-1",
        TurnHostKind::Desktop,
        &ids(&["bot_desktop"]),
        now,
    )
    .await
    .unwrap();
    report_in(
        &db,
        "user-a",
        "desk-1",
        TurnHostKind::Desktop,
        &ids(&["bot_stale"]),
        now - Duration::seconds(151),
    )
    .await
    .unwrap();

    let presence = presence_in(&db, "user-a", now).await.unwrap();
    assert_eq!(
        presence.keys().map(String::as_str).collect::<Vec<_>>(),
        ["bot_desktop", "bot_device", "bot_server", "bot_stale"]
    );
    assert_eq!(state(&presence, "bot_server"), PresenceState::Online);
    assert_eq!(state(&presence, "bot_desktop"), PresenceState::Online);
    assert_eq!(state(&presence, "bot_stale"), PresenceState::Offline);
    assert_eq!(state(&presence, "bot_device"), PresenceState::Unknown);
    assert_eq!(
        presence["bot_desktop"].last_seen_at.as_deref(),
        Some(now.to_rfc3339().as_str())
    );
}

#[tokio::test]
async fn the_newest_of_several_hosts_decides() {
    let db = database().await;
    bots(&db).await;
    let now = at(10_000);
    let stale = now - Duration::seconds(400);
    report_in(
        &db,
        "user-a",
        "desk-old",
        TurnHostKind::Desktop,
        &ids(&["bot_desktop"]),
        stale,
    )
    .await
    .unwrap();
    assert_eq!(
        state(
            &presence_in(&db, "user-a", now).await.unwrap(),
            "bot_desktop"
        ),
        PresenceState::Offline
    );
    report_in(
        &db,
        "user-a",
        "desk-new",
        TurnHostKind::Desktop,
        &ids(&["bot_desktop"]),
        now,
    )
    .await
    .unwrap();
    assert_eq!(
        state(
            &presence_in(&db, "user-a", now).await.unwrap(),
            "bot_desktop"
        ),
        PresenceState::Online
    );
}

#[tokio::test]
async fn a_report_refuses_foreign_deleted_unknown_and_wrong_kind_bots() {
    let db = database().await;
    bots(&db).await;
    insert_bot(&db, "bot_gone", "user-a", "DESKTOP").await;
    delete_bot(&db, "bot_gone").await;
    let now = at(10_000);

    let result = report_in(
        &db,
        "user-a",
        "desk-1",
        TurnHostKind::Desktop,
        &ids(&[
            "bot_desktop",
            "bot_theirs",
            "bot_gone",
            "bot_missing",
            "bot_device",
            "bot_server",
        ]),
        now,
    )
    .await
    .unwrap();
    assert_eq!(result.accepted, ["bot_desktop"]);
    assert_eq!(
        result.refused,
        [
            "bot_device",
            "bot_gone",
            "bot_missing",
            "bot_server",
            "bot_theirs"
        ]
    );

    let presence = presence_in(&db, "user-a", now).await.unwrap();
    assert_eq!(state(&presence, "bot_device"), PresenceState::Unknown);
    let theirs = presence_in(&db, "user-b", now).await.unwrap();
    assert_eq!(state(&theirs, "bot_theirs"), PresenceState::Unknown);

    let device = report_in(
        &db,
        "user-a",
        "dev-1",
        TurnHostKind::Device,
        &ids(&["bot_desktop", "bot_device"]),
        now,
    )
    .await
    .unwrap();
    assert_eq!(device.accepted, ["bot_device"]);
    assert_eq!(device.refused, ["bot_desktop"]);
}

#[tokio::test]
async fn a_desktop_may_report_flux_though_it_runs_on_the_server() {
    let db = database().await;
    bots(&db).await;
    insert_flux(&db, "bot_flux", "user-a").await;
    insert_flux(&db, "bot_their_flux", "user-b").await;
    let now = at(10_000);
    let wanted = ids(&["bot_flux", "bot_server", "bot_their_flux"]);
    let reported = |kind| report_in(&db, "user-a", "host-1", kind, &wanted, now);

    let desktop = reported(TurnHostKind::Desktop).await.unwrap();
    assert_eq!(desktop.accepted, ["bot_flux"]);
    assert_eq!(desktop.refused, ["bot_server", "bot_their_flux"]);
    let device = reported(TurnHostKind::Device).await.unwrap();
    assert!(device.accepted.is_empty(), "{device:?}");

    let presence = presence_in(&db, "user-a", now).await.unwrap();
    assert_eq!(state(&presence, "bot_flux"), PresenceState::Online);
}

#[tokio::test]
async fn reporting_again_moves_the_time_without_a_second_row() {
    let db = database().await;
    bots(&db).await;
    let first = at(10_000);
    let later = first + Duration::seconds(60);
    for now in [first, later, later] {
        report_in(
            &db,
            "user-a",
            "desk-1",
            TurnHostKind::Desktop,
            &ids(&["bot_desktop", "bot_desktop"]),
            now,
        )
        .await
        .unwrap();
    }
    let presence = presence_in(&db, "user-a", later).await.unwrap();
    assert_eq!(
        presence["bot_desktop"].last_seen_at.as_deref(),
        Some(later.to_rfc3339().as_str())
    );
    let empty = report_in(&db, "user-a", "desk-1", TurnHostKind::Desktop, &[], later)
        .await
        .unwrap();
    assert!(empty.accepted.is_empty() && empty.refused.is_empty());
}

#[tokio::test]
async fn forgetting_a_host_removes_only_its_reports_for_the_caller() {
    let db = database().await;
    bots(&db).await;
    let now = at(10_000);
    report_in(
        &db,
        "user-a",
        "desk-1",
        TurnHostKind::Desktop,
        &ids(&["bot_desktop"]),
        now,
    )
    .await
    .unwrap();
    report_in(
        &db,
        "user-a",
        "desk-2",
        TurnHostKind::Desktop,
        &ids(&["bot_desktop"]),
        now,
    )
    .await
    .unwrap();
    report_in(
        &db,
        "user-b",
        "desk-1",
        TurnHostKind::Desktop,
        &ids(&["bot_theirs"]),
        now,
    )
    .await
    .unwrap();

    assert_eq!(forget_in(&db, "user-a", "desk-1").await.unwrap(), 1);
    assert_eq!(forget_in(&db, "user-a", "desk-1").await.unwrap(), 0);
    let mine = presence_in(&db, "user-a", now).await.unwrap();
    assert_eq!(state(&mine, "bot_desktop"), PresenceState::Online);
    let theirs = presence_in(&db, "user-b", now).await.unwrap();
    assert_eq!(state(&theirs, "bot_theirs"), PresenceState::Online);

    assert_eq!(forget_in(&db, "user-a", "desk-2").await.unwrap(), 1);
    let mine = presence_in(&db, "user-a", now).await.unwrap();
    assert_eq!(state(&mine, "bot_desktop"), PresenceState::Unknown);
}
