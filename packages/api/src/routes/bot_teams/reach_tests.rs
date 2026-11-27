use super::{
    access::{TEAM_FORBIDDEN, TEAM_NOT_FOUND},
    bots::{add_bot_in, remove_bot_in},
    classification::set_classification_in,
    lead::{accept_lead_in, decline_lead_in, resign_lead_in},
    model::{AddBotRequest, ClassificationRequest, PatchTeamRequest},
    people::remove_person_in,
    tests::{create, join, own_bot, patch, team, world},
    view::org_view,
};
use crate::{
    entity::{bot, bot_org_classification, bot_team_chat, conversation_member},
    routes::inbox::fixture::{assert_code, at, says},
};
use axum::http::StatusCode;
use flow_like_types::{
    bot_effective::BotRoomLimits,
    bot_team::{TalkReach, TeamInvitationKind},
    data_classification::DataClassification,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    sea_query::Expr,
};

async fn security(db: &DatabaseConnection) -> (String, String) {
    let kestrel = create(db, "mara", "Kestrel", None).await.unwrap();
    let security = create(db, "mara", "Security", Some(&kestrel))
        .await
        .unwrap();
    join(db, "mara", &security, "jonas").await;
    join(db, "mara", &security, "anna").await;
    (kestrel, security)
}

fn lead(bot_id: &str) -> PatchTeamRequest {
    PatchTeamRequest {
        lead_bot_id: Some(Some(bot_id.into())),
        ..Default::default()
    }
}

async fn invite_bot(db: &DatabaseConnection, team_id: &str, bot_id: &str) {
    let request = AddBotRequest {
        bot_id: bot_id.into(),
        limits: None,
    };
    add_bot_in(db, "mara", team_id, &request, at(4))
        .await
        .unwrap();
}

async fn bot_ids_of(db: &DatabaseConnection, viewer: &str, team_id: &str) -> Vec<String> {
    let view = org_view(db, viewer).await.unwrap();
    view.teams
        .into_iter()
        .find(|team| team.id == team_id)
        .unwrap()
        .bots
        .into_iter()
        .map(|bot| bot.bot_id)
        .collect()
}

async fn limits_of(db: &DatabaseConnection, team_id: &str, principal: &str) -> BotRoomLimits {
    let room = team(db, team_id).await.conversation_id.unwrap();
    let member = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(room))
        .filter(conversation_member::Column::PrincipalId.eq(principal))
        .filter(conversation_member::Column::LeftAt.is_null())
        .one(db)
        .await
        .unwrap()
        .unwrap();
    serde_json::from_value(member.limits.unwrap()).unwrap()
}

async fn classify(
    db: &DatabaseConnection,
    sub: &str,
    org: &str,
    user: &str,
    level: Option<DataClassification>,
) -> Result<(), crate::error::ApiError> {
    set_classification_in(
        db,
        sub,
        org,
        user,
        &ClassificationRequest {
            classification: level,
        },
        at(9),
    )
    .await
}

async fn classification_rows(db: &DatabaseConnection) -> usize {
    bot_org_classification::Entity::find()
        .all(db)
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn a_pending_bot_shows_only_to_its_owner_and_the_teams_admins() {
    let db = world().await;
    let (_, security) = security(&db).await;
    invite_bot(&db, &security, "anna-bot").await;
    for viewer in ["anna", "mara"] {
        assert_eq!(bot_ids_of(&db, viewer, &security).await, ["anna-bot"]);
        let view = org_view(&db, viewer).await.unwrap();
        assert!(view.bots.iter().any(|bot| bot.id == "anna-bot"));
    }
    assert!(bot_ids_of(&db, "jonas", &security).await.is_empty());
    let view = org_view(&db, "jonas").await.unwrap();
    assert!(view.bots.iter().all(|bot| bot.id != "anna-bot"));
    assert!(view.reach.iter().all(|reach| reach.bot_id != "anna-bot"));
    let owner = org_view(&db, "anna").await.unwrap();
    let own = owner.reach.iter().find(|r| r.bot_id == "anna-bot").unwrap();
    assert_eq!(own.reach, TalkReach::Own);
    let admin = org_view(&db, "mara").await.unwrap();
    let theirs = admin.reach.iter().find(|r| r.bot_id == "anna-bot").unwrap();
    assert_eq!(theirs.reach, TalkReach::Unavailable);
}

#[tokio::test]
async fn another_owners_lead_is_an_offer_until_the_owner_accepts() {
    let db = world().await;
    let (_, security) = security(&db).await;
    own_bot(&db, "jonas", &security, "jonas-bot").await;
    own_bot(&db, "mara", &security, "mara-bot").await;
    patch(&db, "mara", &security, lead("jonas-bot"))
        .await
        .unwrap();
    let row = team(&db, &security).await;
    assert_eq!(row.lead_bot_id, None);
    assert_eq!(row.lead_offer_bot_id.as_deref(), Some("jonas-bot"));
    assert_eq!(row.lead_offer_by.as_deref(), Some("mara"));

    let offer_of = |viewer: &'static str| {
        let (db, security) = (&db, &security);
        async move {
            org_view(db, viewer)
                .await
                .unwrap()
                .teams
                .into_iter()
                .find(|team| &team.id == security)
                .unwrap()
                .lead_offer_bot_id
        }
    };
    assert_eq!(offer_of("mara").await.as_deref(), Some("jonas-bot"));
    assert_eq!(offer_of("jonas").await.as_deref(), Some("jonas-bot"));
    assert_eq!(offer_of("anna").await, None);
    let invitations = org_view(&db, "jonas").await.unwrap().invitations;
    let invitation = invitations
        .iter()
        .find(|invitation| invitation.kind == TeamInvitationKind::Lead)
        .unwrap();
    assert_eq!(invitation.bot_id.as_deref(), Some("jonas-bot"));
    assert_eq!(invitation.invited_by.as_deref(), Some("mara"));
    assert_eq!(invitation.team_id, security);

    let stranger = accept_lead_in(&db, "mara", &security, "jonas-bot", at(6))
        .await
        .unwrap_err();
    assert_eq!(stranger.status(), StatusCode::NOT_FOUND);

    accept_lead_in(&db, "jonas", &security, "jonas-bot", at(6))
        .await
        .unwrap();
    let row = team(&db, &security).await;
    assert_eq!(row.lead_bot_id.as_deref(), Some("jonas-bot"));
    assert_eq!(row.lead_offer_bot_id, None);
    assert!(!limits_of(&db, &security, "jonas-bot").await.mentioned_only);
    assert!(limits_of(&db, &security, "mara-bot").await.mentioned_only);
    let again = accept_lead_in(&db, "jonas", &security, "jonas-bot", at(7))
        .await
        .unwrap_err();
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn declining_keeps_the_lead_and_resigning_clears_it() {
    let db = world().await;
    let (_, security) = security(&db).await;
    own_bot(&db, "jonas", &security, "jonas-bot").await;
    own_bot(&db, "mara", &security, "mara-bot").await;
    patch(&db, "mara", &security, lead("mara-bot"))
        .await
        .unwrap();
    patch(&db, "mara", &security, lead("jonas-bot"))
        .await
        .unwrap();
    assert_eq!(
        team(&db, &security).await.lead_offer_bot_id.as_deref(),
        Some("jonas-bot")
    );
    decline_lead_in(&db, "jonas", &security, "jonas-bot", at(6))
        .await
        .unwrap();
    let row = team(&db, &security).await;
    assert_eq!(row.lead_bot_id.as_deref(), Some("mara-bot"));
    assert_eq!(row.lead_offer_bot_id, None);
    assert_eq!(
        decline_lead_in(&db, "jonas", &security, "jonas-bot", at(7))
            .await
            .unwrap_err()
            .status(),
        StatusCode::NOT_FOUND
    );

    let not_owner = resign_lead_in(&db, "jonas", &security, "mara-bot", at(8))
        .await
        .unwrap_err();
    assert_eq!(not_owner.status(), StatusCode::NOT_FOUND);
    resign_lead_in(&db, "mara", &security, "mara-bot", at(8))
        .await
        .unwrap();
    assert_eq!(team(&db, &security).await.lead_bot_id, None);
    assert!(limits_of(&db, &security, "mara-bot").await.mentioned_only);
}

#[tokio::test]
async fn removing_the_offered_bot_clears_the_offer_and_an_own_bot_leads_at_once() {
    let db = world().await;
    let (_, security) = security(&db).await;
    own_bot(&db, "jonas", &security, "jonas-bot").await;
    own_bot(&db, "mara", &security, "mara-bot").await;
    patch(&db, "mara", &security, lead("jonas-bot"))
        .await
        .unwrap();
    remove_bot_in(&db, "jonas", &security, "jonas-bot", at(6))
        .await
        .unwrap();
    let row = team(&db, &security).await;
    assert_eq!(row.lead_offer_bot_id, None);
    assert_eq!(row.lead_offer_by, None);
    assert_eq!(row.lead_offer_at, None);

    own_bot(&db, "jonas", &security, "jonas-bot").await;
    patch(&db, "mara", &security, lead("jonas-bot"))
        .await
        .unwrap();
    patch(&db, "mara", &security, lead("mara-bot"))
        .await
        .unwrap();
    let row = team(&db, &security).await;
    assert_eq!(row.lead_bot_id.as_deref(), Some("mara-bot"));
    assert_eq!(row.lead_offer_bot_id, None);

    patch(&db, "mara", &security, lead("jonas-bot"))
        .await
        .unwrap();
    patch(
        &db,
        "mara",
        &security,
        PatchTeamRequest {
            lead_bot_id: Some(None),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let row = team(&db, &security).await;
    assert_eq!((row.lead_bot_id, row.lead_offer_bot_id), (None, None));
}

#[tokio::test]
async fn admins_of_a_top_level_team_classify_the_people_of_the_org() {
    let db = world().await;
    let (kestrel, security) = security(&db).await;
    classify(
        &db,
        "mara",
        &kestrel,
        "jonas",
        Some(DataClassification::Confidential),
    )
    .await
    .unwrap();
    assert_eq!(classification_rows(&db).await, 1);
    classify(
        &db,
        "mara",
        &kestrel,
        "jonas",
        Some(DataClassification::Public),
    )
    .await
    .unwrap();
    let row = bot_org_classification::Entity::find()
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.classification.as_str(), row.set_by.as_str()),
        ("public", "mara")
    );

    let by_member = classify(
        &db,
        "jonas",
        &kestrel,
        "anna",
        Some(DataClassification::Public),
    )
    .await;
    assert_code(
        &by_member.unwrap_err(),
        StatusCode::FORBIDDEN,
        TEAM_FORBIDDEN,
    );
    let below = classify(&db, "mara", &security, "jonas", None)
        .await
        .unwrap_err();
    assert_eq!(below.status(), StatusCode::BAD_REQUEST);
    assert!(says(&below, "Kestrel"), "{below:?}");
    let outsider = classify(
        &db,
        "mara",
        &kestrel,
        "outsider",
        Some(DataClassification::Public),
    )
    .await
    .unwrap_err();
    assert_eq!(outsider.status(), StatusCode::NOT_FOUND);
    let stranger = classify(&db, "outsider", &kestrel, "jonas", None)
        .await
        .unwrap_err();
    assert_code(&stranger, StatusCode::NOT_FOUND, TEAM_NOT_FOUND);

    let mine = org_view(&db, "mara").await.unwrap().classifications;
    assert_eq!(mine.len(), 1);
    assert!(
        org_view(&db, "anna")
            .await
            .unwrap()
            .classifications
            .is_empty()
    );
    let own = org_view(&db, "jonas").await.unwrap().classifications;
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].classification, DataClassification::Public);

    classify(&db, "mara", &kestrel, "jonas", None)
        .await
        .unwrap();
    assert_eq!(classification_rows(&db).await, 0);
}

#[tokio::test]
async fn a_classification_goes_when_the_person_leaves_or_the_team_gets_a_parent() {
    let db = world().await;
    let (kestrel, security) = security(&db).await;
    classify(
        &db,
        "mara",
        &kestrel,
        "jonas",
        Some(DataClassification::Public),
    )
    .await
    .unwrap();
    remove_person_in(&db, "mara", &security, "jonas", at(6))
        .await
        .unwrap();
    assert_eq!(classification_rows(&db).await, 0);

    let other = create(&db, "mara", "Other", None).await.unwrap();
    join(&db, "mara", &other, "anna").await;
    classify(
        &db,
        "mara",
        &other,
        "anna",
        Some(DataClassification::Restricted),
    )
    .await
    .unwrap();
    patch(
        &db,
        "mara",
        &other,
        PatchTeamRequest {
            parent_id: Some(Some(kestrel.clone())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(classification_rows(&db).await, 0);
}

#[tokio::test]
async fn the_org_view_reports_reach_chats_and_gated_bots() {
    let db = world().await;
    let (kestrel, security) = security(&db).await;
    let finance = create(&db, "mara", "Finance", Some(&kestrel))
        .await
        .unwrap();
    join(&db, "mara", &finance, "anna").await;
    own_bot(&db, "anna", &finance, "anna-bot").await;
    own_bot(&db, "jonas", &security, "jonas-bot").await;
    own_bot(&db, "mara", &security, "mara-bot").await;
    bot::Entity::update_many()
        .col_expr(bot::Column::Classification, Expr::value("confidential"))
        .filter(bot::Column::Id.eq("jonas-bot"))
        .exec(&db)
        .await
        .unwrap();

    let view = org_view(&db, "mara").await.unwrap();
    let security_view = view.teams.iter().find(|t| t.id == security).unwrap();
    assert_eq!(security_view.gated_bots, ["jonas-bot"]);
    let reach = |id: &str| view.reach.iter().find(|r| r.bot_id == id).unwrap().reach;
    assert_eq!(reach("mara-bot"), TalkReach::Own);
    assert_eq!(reach("jonas-bot"), TalkReach::Classified);
    let jonas = org_view(&db, "jonas").await.unwrap();
    let jonas_reach = |id: &str| jonas.reach.iter().find(|r| r.bot_id == id).unwrap().reach;
    assert_eq!(jonas_reach("anna-bot"), TalkReach::Request);
    assert_eq!(jonas_reach("mara-bot"), TalkReach::TeamRoom);

    bot_team_chat::ActiveModel {
        bot_id: Set("anna-bot".into()),
        user_id: Set("jonas".into()),
        owner_id: Set("anna".into()),
        status: Set("PENDING".into()),
        conversation_id: Set(None),
        limits: Set(None),
        note: Set(Some("budget question".into())),
        requested_at: Set(at(10)),
        decided_at: Set(None),
        ended_at: Set(None),
    }
    .insert(&db)
    .await
    .unwrap();
    let asker = org_view(&db, "jonas").await.unwrap();
    assert_eq!(
        asker
            .reach
            .iter()
            .find(|r| r.bot_id == "anna-bot")
            .unwrap()
            .reach,
        TalkReach::Requested
    );
    assert_eq!(asker.chats.len(), 1);
    let anna = org_view(&db, "anna").await.unwrap();
    assert_eq!(anna.chats.len(), 1);
    let request = anna
        .invitations
        .iter()
        .find(|i| i.kind == TeamInvitationKind::ChatRequest)
        .unwrap();
    assert_eq!(request.user_id.as_deref(), Some("jonas"));
    assert_eq!(request.note.as_deref(), Some("budget question"));
    assert_eq!(request.team_id, finance);
    assert_eq!(request.org_name, "Kestrel");
    assert!(org_view(&db, "mara").await.unwrap().chats.is_empty());
}
