use super::{Fixture, create, fixture};
use crate::{Bot, SkillOrigin};
use flow_like_types::bot_skill::{CreateBotSkill, SkillSource, SkillStatus, UpdateBotSkill};
use serde_json::json;

fn skilled_bot(f: &Fixture) -> Bot {
    f.store
        .create_bot(create(json!({
            "name": "Helper",
            "appearance": { "kind": "pip" },
            "capabilities": ["skills"],
        })))
        .unwrap()
}

fn request(name: &str) -> CreateBotSkill {
    CreateBotSkill {
        name: name.into(),
        description: format!("About {name}"),
        body: "1. Do it".into(),
        tools: Vec::new(),
    }
}

#[test]
fn a_skill_needs_the_capability_and_valid_text() {
    let f = fixture();
    let plain = f
        .store
        .create_bot(create(json!({ "name": "Plain", "appearance": { "kind": "pip" } })))
        .unwrap();
    let error = f
        .store
        .create_skill(&plain.id, request("a"), SkillOrigin::owner())
        .unwrap_err();
    assert!(error.to_string().contains("skills capability"), "{error}");

    let bot = skilled_bot(&f);
    for bad in [
        request("Bad Name"),
        CreateBotSkill {
            body: "hide\u{200B}this".into(),
            ..request("ok")
        },
    ] {
        assert!(f.store.create_skill(&bot.id, bad, SkillOrigin::owner()).is_err());
    }
    assert!(f.store.skills(&bot.id).unwrap().is_empty());
}

#[test]
fn a_pending_skill_is_never_in_the_index_until_approved() {
    let f = fixture();
    let bot = skilled_bot(&f);
    let proposed = f
        .store
        .create_skill(&bot.id, request("weekly"), SkillOrigin::proposed_by_bot())
        .unwrap();
    assert_eq!(proposed.status, SkillStatus::Pending);
    assert_eq!(proposed.source, SkillSource::BotProposed);
    assert!(f.store.active_skill_index(&bot.id).unwrap().is_empty());

    f.store
        .create_skill(&bot.id, request("mine"), SkillOrigin::owner())
        .unwrap();
    let names: Vec<_> = f
        .store
        .active_skill_index(&bot.id)
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(names, ["mine"]);

    f.store
        .set_skill_status(&bot.id, &proposed.id, SkillStatus::Active)
        .unwrap();
    assert_eq!(f.store.active_skill_index(&bot.id).unwrap().len(), 2);
    f.store
        .set_skill_status(&bot.id, &proposed.id, SkillStatus::Disabled)
        .unwrap();
    assert_eq!(f.store.active_skill_index(&bot.id).unwrap().len(), 1);
}

#[test]
fn names_are_unique_and_reimport_puts_a_skill_back_to_pending() {
    let f = fixture();
    let bot = skilled_bot(&f);
    let first = f
        .store
        .create_skill(&bot.id, request("pdf"), SkillOrigin::owner())
        .unwrap();
    assert!(
        f.store
            .create_skill(&bot.id, request("pdf"), SkillOrigin::owner())
            .is_err()
    );
    let again = f
        .store
        .reimport_skill(
            &bot.id,
            CreateBotSkill {
                body: "new text".into(),
                ..request("pdf")
            },
        )
        .unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.status, SkillStatus::Pending);
    assert_eq!(again.body, "new text");
    assert!(f.store.active_skill_index(&bot.id).unwrap().is_empty());
}

#[test]
fn skills_update_delete_and_go_with_their_bot() {
    let f = fixture();
    let bot = skilled_bot(&f);
    let skill = f
        .store
        .create_skill(&bot.id, request("pdf"), SkillOrigin::owner())
        .unwrap();
    let updated = f
        .store
        .update_skill(
            &bot.id,
            &skill.id,
            UpdateBotSkill {
                body: Some("changed".into()),
                tools: Some(vec!["extract".into(), "extract".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(updated.body, "changed");
    assert_eq!(updated.tools, ["extract"]);
    assert_eq!(updated.status, SkillStatus::Active);
    assert!(f.store.update_skill(&bot.id, "missing", UpdateBotSkill::default()).is_err());
    assert!(
        f.store
            .update_skill(
                &bot.id,
                &skill.id,
                UpdateBotSkill {
                    description: Some("a\nb".into()),
                    ..Default::default()
                }
            )
            .is_err()
    );
    let removed = f.store.delete_skill(&bot.id, &skill.id).unwrap();
    assert_eq!(removed.name, "pdf");
    assert!(f.store.skills(&bot.id).unwrap().is_empty());
}

#[test]
fn another_bots_skill_reads_as_missing() {
    let f = fixture();
    let one = skilled_bot(&f);
    let two = skilled_bot(&f);
    let skill = f
        .store
        .create_skill(&one.id, request("pdf"), SkillOrigin::owner())
        .unwrap();
    assert!(f.store.set_skill_status(&two.id, &skill.id, SkillStatus::Disabled).is_err());
    assert!(f.store.delete_skill(&two.id, &skill.id).is_err());
    assert!(f.store.skill_by_name(&two.id, "pdf").unwrap().is_none());
}
