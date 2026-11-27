use super::{
    access::{TEAM_CYCLE, TEAM_FORBIDDEN, TEAM_LAST_ADMIN, TEAM_NOT_FOUND, visible_team},
    bots::{
        accept_bot_in, add_bot_in, decline_bot_in, may_read_briefing, remove_bot_in, set_direct_in,
    },
    lead::accept_lead_in,
    model::{
        AcceptBotRequest, AddBotRequest, AddPeopleRequest, CreateTeamRequest, PatchTeamRequest,
    },
    people::{add_people_in, decline_in, join_in, remove_person_in, set_role_in},
    teams::{create_team_in, delete_team_in, patch_team_in},
    view::org_view,
};
use crate::{
    entity::{bot, bot_team, bot_team_bot, conversation_member},
    routes::inbox::fixture::{assert_code, at, database, insert_bot, insert_flux},
};
use axum::http::StatusCode;
use flow_like_types::{
    bot_effective::BotRoomLimits,
    bot_team::{TeamInvitationKind, TeamRole},
};
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, Schema};
use std::collections::BTreeMap;

pub(super) async fn world() -> DatabaseConnection {
    let db = database().await;
    let schema = Schema::new(db.get_database_backend());
    for table in [
        schema.create_table_from_entity(crate::entity::bot_team_person::Entity),
        schema.create_table_from_entity(bot_team_bot::Entity),
    ] {
        db.execute(&table).await.unwrap();
    }
    db.execute_unprepared(
        r#"INSERT INTO public."User" ("id", "name", "email", "username", "preferredUsername") VALUES
            ('mara', 'Mara', 'mara@example.org', 'g1', 'mara'),
            ('jonas', 'Jonas', 'jonas@example.org', 'g2', 'jonas'),
            ('anna', 'Anna', 'anna@example.org', 'g3', 'anna'),
            ('outsider', 'Out', 'out@example.org', 'g4', 'outsider')"#,
    )
    .await
    .unwrap();
    insert_bot(&db, "mara-bot", "mara", "SERVER").await;
    insert_bot(&db, "mara-bot-2", "mara", "SERVER").await;
    insert_bot(&db, "jonas-bot", "jonas", "SERVER").await;
    insert_bot(&db, "jonas-bot-2", "jonas", "SERVER").await;
    insert_bot(&db, "anna-bot", "anna", "SERVER").await;
    insert_flux(&db, "mara-flux", "mara").await;
    db
}

pub(super) async fn create(
    db: &DatabaseConnection,
    sub: &str,
    name: &str,
    parent: Option<&str>,
) -> Result<String, crate::error::ApiError> {
    let request = CreateTeamRequest {
        name: name.into(),
        purpose: None,
        parent_id: parent.map(str::to_string),
    };
    create_team_in(db, sub, &request, at(1))
        .await
        .map(|(id, _)| id)
}

pub(super) async fn join(db: &DatabaseConnection, admin: &str, team: &str, user: &str) {
    let request = AddPeopleRequest {
        user_ids: vec![user.into()],
        role: None,
    };
    add_people_in(db, admin, team, &request, at(2))
        .await
        .unwrap();
    join_in(db, user, team, at(3)).await.unwrap();
}

pub(super) async fn own_bot(db: &DatabaseConnection, sub: &str, team: &str, bot_id: &str) {
    let request = AddBotRequest {
        bot_id: bot_id.into(),
        limits: None,
    };
    add_bot_in(db, sub, team, &request, at(4)).await.unwrap();
}

pub(super) async fn patch(
    db: &DatabaseConnection,
    sub: &str,
    team: &str,
    request: PatchTeamRequest,
) -> Result<(), crate::error::ApiError> {
    patch_team_in(db, sub, team, &request, at(5))
        .await
        .map(|_| ())
}

pub(super) async fn team(db: &DatabaseConnection, id: &str) -> bot_team::Model {
    bot_team::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

async fn room(db: &DatabaseConnection, team_id: &str) -> BTreeMap<String, Option<BotRoomLimits>> {
    let conversation = team(db, team_id).await.conversation_id.unwrap();
    conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conversation))
        .filter(conversation_member::Column::LeftAt.is_null())
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|member| {
            let limits = member
                .limits
                .and_then(|value| serde_json::from_value(value).ok());
            (member.principal_id, limits)
        })
        .collect()
}

async fn kestrel_security(db: &DatabaseConnection) -> (String, String) {
    let kestrel = create(db, "mara", "Kestrel", None).await.unwrap();
    let security = create(db, "mara", "Security", Some(&kestrel))
        .await
        .unwrap();
    join(db, "mara", &security, "jonas").await;
    (kestrel, security)
}

#[tokio::test]
async fn a_member_of_a_sub_team_sees_the_whole_org_and_an_outsider_nothing() {
    let db = world().await;
    let (kestrel, security) = kestrel_security(&db).await;
    own_bot(&db, "mara", &kestrel, "mara-bot").await;

    let view = org_view(&db, "jonas").await.unwrap();
    let ids: Vec<&str> = view.teams.iter().map(|team| team.id.as_str()).collect();
    assert_eq!(ids, [kestrel.as_str(), security.as_str()]);
    assert!(view.bots.iter().any(|bot| bot.id == "mara-bot"));

    assert!(org_view(&db, "outsider").await.unwrap().teams.is_empty());
    let hidden = visible_team(&db, "outsider", &kestrel).await.err().unwrap();
    assert_code(&hidden, StatusCode::NOT_FOUND, TEAM_NOT_FOUND);
    let not_admin = patch(&db, "jonas", &kestrel, PatchTeamRequest::default())
        .await
        .unwrap_err();
    assert_code(&not_admin, StatusCode::FORBIDDEN, TEAM_FORBIDDEN);
}

#[tokio::test]
async fn a_sub_team_needs_an_admin_of_its_parent() {
    let db = world().await;
    let (kestrel, security) = kestrel_security(&db).await;
    let member = create(&db, "jonas", "Audit", Some(&security))
        .await
        .unwrap_err();
    assert_code(&member, StatusCode::FORBIDDEN, TEAM_FORBIDDEN);
    let stranger = create(&db, "outsider", "Audit", Some(&kestrel))
        .await
        .unwrap_err();
    assert_code(&stranger, StatusCode::NOT_FOUND, TEAM_NOT_FOUND);
    assert!(create(&db, "mara", "Audit", Some(&security)).await.is_ok());
}

#[tokio::test]
async fn people_invite_join_decline_and_leave() {
    let db = world().await;
    let kestrel = create(&db, "mara", "Kestrel", None).await.unwrap();
    let request = AddPeopleRequest {
        user_ids: vec!["anna".into(), "jonas".into()],
        role: Some(TeamRole::Member),
    };
    add_people_in(&db, "mara", &kestrel, &request, at(2))
        .await
        .unwrap();
    let view = org_view(&db, "anna").await.unwrap();
    assert!(view.teams.is_empty());
    assert_eq!(view.invitations[0].kind, TeamInvitationKind::Person);
    assert_eq!(view.invitations[0].team_name, "Kestrel");
    assert_eq!(view.invitations[0].org_name, "Kestrel");

    decline_in(&db, "anna", &kestrel).await.unwrap();
    assert!(org_view(&db, "anna").await.unwrap().invitations.is_empty());
    join_in(&db, "jonas", &kestrel, at(3)).await.unwrap();
    assert!(room(&db, &kestrel).await.contains_key("jonas"));

    let missing = AddPeopleRequest {
        user_ids: vec!["jonas".into(), "ghost".into()],
        role: None,
    };
    let error = add_people_in(&db, "mara", &kestrel, &missing, at(4))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);

    remove_person_in(&db, "jonas", &kestrel, "jonas", at(5))
        .await
        .unwrap();
    assert!(!room(&db, &kestrel).await.contains_key("jonas"));
}

#[tokio::test]
async fn the_last_active_admin_stays_while_others_remain() {
    let db = world().await;
    let kestrel = create(&db, "mara", "Kestrel", None).await.unwrap();
    join(&db, "mara", &kestrel, "jonas").await;
    let demote = set_role_in(&db, "mara", &kestrel, "mara", TeamRole::Member)
        .await
        .unwrap_err();
    assert_code(&demote, StatusCode::CONFLICT, TEAM_LAST_ADMIN);
    let leave = remove_person_in(&db, "mara", &kestrel, "mara", at(5))
        .await
        .unwrap_err();
    assert_code(&leave, StatusCode::CONFLICT, TEAM_LAST_ADMIN);
    set_role_in(&db, "mara", &kestrel, "jonas", TeamRole::Admin)
        .await
        .unwrap();
    remove_person_in(&db, "mara", &kestrel, "mara", at(6))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_own_bot_joins_at_once_and_moving_teams_clears_the_old_lead() {
    let db = world().await;
    let (kestrel, security) = kestrel_security(&db).await;
    own_bot(&db, "mara", &kestrel, "mara-bot").await;
    let lead = PatchTeamRequest {
        lead_bot_id: Some(Some("mara-bot".into())),
        ..Default::default()
    };
    patch(&db, "mara", &kestrel, lead).await.unwrap();
    assert_eq!(
        team(&db, &kestrel).await.lead_bot_id.as_deref(),
        Some("mara-bot")
    );

    own_bot(&db, "mara", &security, "mara-bot").await;
    assert_eq!(team(&db, &kestrel).await.lead_bot_id, None);
    assert!(!room(&db, &kestrel).await.contains_key("mara-bot"));
    assert!(room(&db, &security).await.contains_key("mara-bot"));
    let rows = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::BotId.eq("mara-bot"))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].team_id, security);
}

#[tokio::test]
async fn an_admin_invites_a_members_bot_and_its_owner_decides() {
    let db = world().await;
    let (kestrel, security) = kestrel_security(&db).await;
    let invite = AddBotRequest {
        bot_id: "jonas-bot".into(),
        limits: None,
    };
    let early = add_bot_in(
        &db,
        "jonas",
        &security,
        &AddBotRequest {
            bot_id: "mara-bot".into(),
            limits: None,
        },
        at(4),
    )
    .await;
    assert_code(&early.unwrap_err(), StatusCode::FORBIDDEN, TEAM_FORBIDDEN);

    let effects = add_bot_in(&db, "mara", &security, &invite, at(4))
        .await
        .unwrap();
    assert_eq!(effects.notices.len(), 1);
    assert_eq!(effects.notices[0].user_id, "jonas");
    let view = org_view(&db, "jonas").await.unwrap();
    assert!(view.invitations.iter().any(|invitation| {
        invitation.kind == TeamInvitationKind::Bot
            && invitation.bot_id.as_deref() == Some("jonas-bot")
    }));
    assert!(!room(&db, &security).await.contains_key("jonas-bot"));

    let limits = AcceptBotRequest {
        limits: Some(BotRoomLimits {
            apps: Some(vec!["app-1".into()]),
            ..Default::default()
        }),
    };
    let stranger = accept_bot_in(&db, "mara", &security, "jonas-bot", &limits, at(5)).await;
    assert_eq!(stranger.unwrap_err().status(), StatusCode::NOT_FOUND);
    accept_bot_in(&db, "jonas", &security, "jonas-bot", &limits, at(5))
        .await
        .unwrap();
    let held = room(&db, &security).await["jonas-bot"].clone().unwrap();
    assert_eq!(held.apps, Some(vec!["app-1".to_string()]));
    assert!(held.mentioned_only);

    let outsider_bot = AddBotRequest {
        bot_id: "anna-bot".into(),
        limits: None,
    };
    let error = add_bot_in(&db, "mara", &kestrel, &outsider_bot, at(6))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);

    add_bot_in(&db, "mara", &kestrel, &invite, at(6))
        .await
        .unwrap();
    decline_bot_in(&db, "jonas", &kestrel, "jonas-bot")
        .await
        .unwrap();
    let rows = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::BotId.eq("jonas-bot"))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].team_id, security);
}

#[tokio::test]
async fn flux_and_deleted_bots_can_not_join() {
    let db = world().await;
    let kestrel = create(&db, "mara", "Kestrel", None).await.unwrap();
    let flux = AddBotRequest {
        bot_id: "mara-flux".into(),
        limits: None,
    };
    let error = add_bot_in(&db, "mara", &kestrel, &flux, at(4))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    crate::routes::inbox::fixture::delete_bot(&db, "mara-bot-2").await;
    let gone = AddBotRequest {
        bot_id: "mara-bot-2".into(),
        limits: None,
    };
    let error = add_bot_in(&db, "mara", &kestrel, &gone, at(4))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn direct_asks_follow_the_switch_and_each_owners_agreement() {
    let db = world().await;
    let (kestrel, security) = kestrel_security(&db).await;
    own_bot(&db, "mara", &security, "mara-bot").await;
    add_bot_in(
        &db,
        "mara",
        &security,
        &AddBotRequest {
            bot_id: "jonas-bot".into(),
            limits: None,
        },
        at(4),
    )
    .await
    .unwrap();
    accept_bot_in(
        &db,
        "jonas",
        &security,
        "jonas-bot",
        &AcceptBotRequest::default(),
        at(5),
    )
    .await
    .unwrap();

    let agreed = |bot_id: &'static str| {
        let db = &db;
        let security = &security;
        async move {
            bot_team_bot::Entity::find_by_id((security.clone(), bot_id.to_string()))
                .one(db)
                .await
                .unwrap()
                .unwrap()
                .direct_agreed
        }
    };
    let on = PatchTeamRequest {
        direct_asks: Some(true),
        ..Default::default()
    };
    patch(&db, "mara", &security, on).await.unwrap();
    assert!(agreed("mara-bot").await);
    assert!(!agreed("jonas-bot").await);
    set_direct_in(&db, "jonas", &security, "jonas-bot", true)
        .await
        .unwrap();
    assert!(agreed("jonas-bot").await);
    let stranger = set_direct_in(&db, "mara", &security, "jonas-bot", false).await;
    assert_eq!(stranger.unwrap_err().status(), StatusCode::NOT_FOUND);

    let off = PatchTeamRequest {
        direct_asks: Some(false),
        ..Default::default()
    };
    patch(&db, "mara", &security, off).await.unwrap();
    assert!(!agreed("mara-bot").await && !agreed("jonas-bot").await);
    let _ = kestrel;
}

#[tokio::test]
async fn a_team_cannot_move_into_itself_or_below() {
    let db = world().await;
    let (kestrel, security) = kestrel_security(&db).await;
    let audit = create(&db, "mara", "Audit", Some(&security)).await.unwrap();
    for target in [&security, &audit, &kestrel] {
        let into = PatchTeamRequest {
            parent_id: Some(Some(target.clone())),
            ..Default::default()
        };
        let error = patch(&db, "mara", &kestrel, into).await.unwrap_err();
        assert_code(&error, StatusCode::BAD_REQUEST, TEAM_CYCLE);
    }
    let up = PatchTeamRequest {
        parent_id: Some(None),
        ..Default::default()
    };
    patch(&db, "mara", &audit, up).await.unwrap();
    assert_eq!(team(&db, &audit).await.parent_id, None);
}

#[tokio::test]
async fn deleting_a_team_moves_its_children_up() {
    let db = world().await;
    let (kestrel, security) = kestrel_security(&db).await;
    let audit = create(&db, "mara", "Audit", Some(&security)).await.unwrap();
    let member = delete_team_in(&db, "jonas", &security, at(8))
        .await
        .unwrap_err();
    assert_code(&member, StatusCode::FORBIDDEN, TEAM_FORBIDDEN);
    delete_team_in(&db, "mara", &security, at(8)).await.unwrap();
    assert_eq!(
        team(&db, &audit).await.parent_id.as_deref(),
        Some(kestrel.as_str())
    );
    assert!(
        bot_team::Entity::find_by_id(security)
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn the_lead_reads_along_and_other_bots_answer_when_named() {
    let db = world().await;
    let (_, security) = kestrel_security(&db).await;
    own_bot(&db, "mara", &security, "mara-bot").await;
    own_bot(&db, "jonas", &security, "jonas-bot").await;
    let members = room(&db, &security).await;
    assert!(
        ["mara", "jonas", "mara-bot", "jonas-bot"]
            .iter()
            .all(|id| members.contains_key(*id))
    );
    let mentioned = |members: &BTreeMap<String, Option<BotRoomLimits>>, id: &str| {
        members[id]
            .as_ref()
            .is_some_and(|limits| limits.mentioned_only)
    };
    assert!(mentioned(&members, "mara-bot") && mentioned(&members, "jonas-bot"));

    let lead = PatchTeamRequest {
        lead_bot_id: Some(Some("jonas-bot".into())),
        ..Default::default()
    };
    patch(&db, "mara", &security, lead).await.unwrap();
    accept_lead_in(&db, "jonas", &security, "jonas-bot", at(5))
        .await
        .unwrap();
    let members = room(&db, &security).await;
    assert!(!mentioned(&members, "jonas-bot") && mentioned(&members, "mara-bot"));

    let stranger = PatchTeamRequest {
        lead_bot_id: Some(Some("anna-bot".into())),
        ..Default::default()
    };
    assert_eq!(
        patch(&db, "mara", &security, stranger)
            .await
            .unwrap_err()
            .status(),
        StatusCode::BAD_REQUEST
    );

    let rename = PatchTeamRequest {
        name: Some("Sec Ops".into()),
        ..Default::default()
    };
    patch(&db, "mara", &security, rename).await.unwrap();
    let conversation = team(&db, &security).await.conversation_id.unwrap();
    let title = crate::entity::conversation::Entity::find_by_id(conversation)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .title;
    assert_eq!(title.as_deref(), Some("Sec Ops"));

    remove_bot_in(&db, "jonas", &security, "jonas-bot", at(7))
        .await
        .unwrap();
    assert_eq!(team(&db, &security).await.lead_bot_id, None);
    assert!(!room(&db, &security).await.contains_key("jonas-bot"));
}

#[tokio::test]
async fn a_briefing_is_for_the_owner_and_the_org() {
    let db = world().await;
    let (_, security) = kestrel_security(&db).await;
    own_bot(&db, "mara", &security, "mara-bot").await;
    let row = bot::Entity::find_by_id("mara-bot")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(may_read_briefing(&db, "mara", &row).await.unwrap());
    assert!(may_read_briefing(&db, "jonas", &row).await.unwrap());
    assert!(!may_read_briefing(&db, "outsider", &row).await.unwrap());
}

#[tokio::test]
async fn leaving_the_org_takes_the_persons_bots_along() {
    let db = world().await;
    let (_, security) = kestrel_security(&db).await;
    own_bot(&db, "jonas", &security, "jonas-bot").await;
    remove_person_in(&db, "jonas", &security, "jonas", at(7))
        .await
        .unwrap();
    let rows = bot_team_bot::Entity::find().all(&db).await.unwrap();
    assert!(rows.is_empty());
}

/// Jonas is only in Security; two of his bots sit in Finance, one pending and one active.
async fn bots_in_finance(db: &DatabaseConnection) -> (String, String, String) {
    let (kestrel, security) = kestrel_security(db).await;
    let finance = create(db, "mara", "Finance", Some(&kestrel)).await.unwrap();
    for bot_id in ["jonas-bot", "jonas-bot-2"] {
        let invite = AddBotRequest {
            bot_id: bot_id.into(),
            limits: None,
        };
        add_bot_in(db, "mara", &finance, &invite, at(4))
            .await
            .unwrap();
    }
    accept_bot_in(
        db,
        "jonas",
        &finance,
        "jonas-bot-2",
        &AcceptBotRequest::default(),
        at(5),
    )
    .await
    .unwrap();
    assert!(room(db, &finance).await.contains_key("jonas-bot-2"));
    (kestrel, security, finance)
}

async fn rows_of_jonas(db: &DatabaseConnection) -> usize {
    bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::BotId.is_in(["jonas-bot", "jonas-bot-2"]))
        .all(db)
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn removing_a_person_prunes_their_bots_in_other_teams_of_the_org() {
    let db = world().await;
    let (_, security, finance) = bots_in_finance(&db).await;
    remove_person_in(&db, "jonas", &security, "jonas", at(7))
        .await
        .unwrap();
    assert_eq!(rows_of_jonas(&db).await, 0);
    assert!(!room(&db, &finance).await.contains_key("jonas-bot-2"));
}

#[tokio::test]
async fn deleting_a_team_prunes_bots_of_people_who_left_the_org_with_it() {
    let db = world().await;
    let (_, security, finance) = bots_in_finance(&db).await;
    delete_team_in(&db, "mara", &security, at(7)).await.unwrap();
    assert_eq!(rows_of_jonas(&db).await, 0);
    assert!(!room(&db, &finance).await.contains_key("jonas-bot-2"));
}

#[tokio::test]
async fn deleting_the_root_splits_the_org_and_prunes_each_part() {
    let db = world().await;
    let (kestrel, _, finance) = bots_in_finance(&db).await;
    delete_team_in(&db, "mara", &kestrel, at(7)).await.unwrap();
    assert_eq!(team(&db, &finance).await.parent_id, None);
    assert_eq!(rows_of_jonas(&db).await, 0);
}

#[tokio::test]
async fn moving_a_team_to_top_level_prunes_both_orgs() {
    let db = world().await;
    let (_, security, finance) = bots_in_finance(&db).await;
    own_bot(&db, "mara", &finance, "mara-bot").await;
    own_bot(&db, "jonas", &security, "jonas-bot").await;
    let split = PatchTeamRequest {
        parent_id: Some(None),
        ..Default::default()
    };
    patch(&db, "mara", &finance, split).await.unwrap();
    let rows = bot_team_bot::Entity::find().all(&db).await.unwrap();
    let mut kept: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r.team_id.as_str(), r.bot_id.as_str()))
        .collect();
    kept.sort();
    let mut expected = [
        (finance.as_str(), "mara-bot"),
        (security.as_str(), "jonas-bot"),
    ];
    expected.sort();
    assert_eq!(kept, expected);
}

#[tokio::test]
async fn an_owner_who_left_the_org_can_not_accept_a_bot_invitation() {
    let db = world().await;
    let (_, security, finance) = bots_in_finance(&db).await;
    add_bot_in(
        &db,
        "mara",
        &finance,
        &AddBotRequest {
            bot_id: "mara-bot".into(),
            limits: None,
        },
        at(5),
    )
    .await
    .unwrap();
    let invite = AddBotRequest {
        bot_id: "jonas-bot".into(),
        limits: None,
    };
    add_bot_in(&db, "mara", &finance, &invite, at(5))
        .await
        .unwrap();
    crate::entity::bot_team_person::Entity::delete_by_id((security, "jonas".to_string()))
        .exec(&db)
        .await
        .unwrap();
    let error = accept_bot_in(
        &db,
        "jonas",
        &finance,
        "jonas-bot",
        &AcceptBotRequest::default(),
        at(6),
    )
    .await
    .unwrap_err();
    assert_code(&error, StatusCode::FORBIDDEN, TEAM_FORBIDDEN);
}

#[test]
fn the_routes_register_without_conflicts() {
    let _ = super::routes();
}
