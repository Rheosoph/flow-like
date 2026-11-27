use super::{
    classification::set_classification_in,
    model::{ClassificationRequest, PatchTeamRequest},
    tests::{create, join, own_bot, patch, team, world},
};
use crate::{
    bot_host::teams::{org_lookup_on, team_brief_on},
    routes::inbox::fixture::{at, group, user},
};
use flow_like_types::{
    bot_team::{BriefPerson, TeamRole, team_brief_text},
    data_classification::DataClassification,
    inbox::ConversationEncryption,
};
use sea_orm::DatabaseConnection;

/// Kestrel, which builds drones, holds Security (Jonas) and Finance (Anna); Mara created all three.
/// Each of them has one bot in their team.
async fn kestrel(db: &DatabaseConnection) -> (String, String) {
    let kestrel = create(db, "mara", "Kestrel", None).await.unwrap();
    let purpose = PatchTeamRequest {
        purpose: Some(Some("Kestrel builds drones for farms".into())),
        ..PatchTeamRequest::default()
    };
    patch(db, "mara", &kestrel, purpose).await.unwrap();
    let security = create(db, "mara", "Security", Some(&kestrel))
        .await
        .unwrap();
    join(db, "mara", &security, "jonas").await;
    let finance = create(db, "mara", "Finance", Some(&kestrel)).await.unwrap();
    join(db, "mara", &finance, "anna").await;
    own_bot(db, "mara", &kestrel, "mara-bot").await;
    own_bot(db, "jonas", &security, "jonas-bot").await;
    own_bot(db, "anna", &finance, "anna-bot").await;
    (kestrel, security)
}

async fn classify(db: &DatabaseConnection, org: &str, person: &str, level: DataClassification) {
    let request = ClassificationRequest {
        classification: Some(level),
    };
    set_classification_in(db, "mara", org, person, &request, at(9))
        .await
        .unwrap();
}

fn named<'a>(people: &'a [BriefPerson], name: &str) -> &'a BriefPerson {
    people.iter().find(|person| person.name == name).unwrap()
}

#[tokio::test]
async fn the_brief_names_the_org_and_what_it_does() {
    let db = world().await;
    kestrel(&db).await;
    let brief = team_brief_on(&db, "jonas-bot", None)
        .await
        .unwrap()
        .unwrap();
    assert!(brief.present.is_empty());
    let text = team_brief_text(&brief);
    assert!(text.contains("You work in Kestrel, an organization of 3 teams and 3 bots."));
    assert!(text.contains("What Kestrel does: Kestrel builds drones for farms."));
    assert!(text.contains("  - Security") && text.contains("(your team)"));
    assert!(text.contains("  - Finance"));
    assert!(!text.contains("## IN THIS CONVERSATION"));
}

#[tokio::test]
async fn in_the_team_room_the_brief_names_the_people_and_only_their_lowest_level() {
    let db = world().await;
    let (kestrel, security) = kestrel(&db).await;
    classify(&db, &kestrel, "mara", DataClassification::Restricted).await;
    classify(&db, &kestrel, "jonas", DataClassification::Confidential).await;
    let room = team(&db, &security).await.conversation_id.unwrap();

    let brief = team_brief_on(&db, "jonas-bot", Some(&room))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(brief.present_level, Some(DataClassification::Confidential));
    assert_eq!(brief.present.len(), 2);
    assert!(brief.present.last().unwrap().owner);
    let mara = named(&brief.present, "Mara");
    assert!(!mara.owner);
    assert!(
        mara.teams
            .iter()
            .any(|membership| membership.team == "Security" && membership.role == TeamRole::Admin)
    );
    let jonas = named(&brief.present, "Jonas");
    assert_eq!(jonas.teams.len(), 1);
    assert_eq!(
        (jonas.teams[0].team.as_str(), jonas.teams[0].role),
        ("Security", TeamRole::Member)
    );
    let people = serde_json::to_string(&brief.present).unwrap();
    assert!(!people.contains("restricted") && !people.contains("confidential"));
    assert!(
        team_brief_text(&brief).contains("the lowest classification among them is confidential")
    );
}

#[tokio::test]
async fn the_lookup_finds_teams_bots_and_people_of_the_org() {
    let db = world().await;
    kestrel(&db).await;
    let finance = org_lookup_on(&db, "jonas-bot", None, "finance")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(finance.org, "Kestrel");
    let team = &finance.teams[0];
    assert_eq!(team.name, "Finance");
    assert_eq!(team.part_of.as_deref(), Some("Kestrel"));
    assert_eq!(team.bots, ["anna-bot, Anna's bot"]);
    assert!(team.people.contains(&"Anna (member)".to_string()));

    let anna = org_lookup_on(&db, "jonas-bot", None, "Anna")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(anna.people[0].teams, ["Finance (member)"]);
    assert_eq!(anna.bots[0].team, "Finance");

    let every = org_lookup_on(&db, "jonas-bot", None, "")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(every.teams.len(), 3);
}

#[tokio::test]
async fn with_an_outsider_present_there_is_no_brief_and_no_lookup() {
    let db = world().await;
    kestrel(&db).await;
    group(
        &db,
        "mixed",
        "jonas",
        &[user("outsider")],
        ConversationEncryption::Plain,
    )
    .await;
    assert!(
        team_brief_on(&db, "jonas-bot", Some("mixed"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        org_lookup_on(&db, "jonas-bot", Some("mixed"), "finance")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        org_lookup_on(&db, "outsider-bot", None, "finance")
            .await
            .unwrap()
            .is_none()
    );
}
