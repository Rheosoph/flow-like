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

#[test]
fn a_bot_takes_its_skills_along_when_it_moves() {
    let f = fixture();
    let bot = skilled_bot(&f);
    f.store
        .create_skill(&bot.id, request("mine"), SkillOrigin::owner())
        .unwrap();
    f.store
        .create_skill(&bot.id, request("draft"), SkillOrigin::proposed_by_bot())
        .unwrap();
    let bundle = f.store.export_bot(&bot.id).unwrap();
    let mut seen: Vec<_> = bundle
        .skills
        .iter()
        .map(|skill| (skill.name.as_str(), skill.status))
        .collect();
    seen.sort_by_key(|(name, _)| *name);
    assert_eq!(
        seen,
        [("draft", SkillStatus::Pending), ("mine", SkillStatus::Active)]
    );
}

fn package_entry(name: &str) -> CreateBotSkill {
    CreateBotSkill {
        tools: vec!["fill_form".into()],
        ..request(name)
    }
}

#[test]
fn a_package_installs_pending_skills_only_when_the_bot_covers_its_tags() {
    let f = fixture();
    let bot = skilled_bot(&f);
    let tags = vec!["net.http".to_string(), "storage.node".to_string()];
    let error = f
        .store
        .install_package_skills(&bot.id, "com.acme.pdf", "1.0.0", vec![package_entry("pdf-forms")], &tags, true)
        .unwrap_err();
    assert!(error.to_string().contains("internet"), "{error}");
    assert!(f.store.skills(&bot.id).unwrap().is_empty());

    let online = f
        .store
        .create_bot(create(json!({
            "name": "Online",
            "appearance": { "kind": "pip" },
            "capabilities": ["skills", "internet"],
        })))
        .unwrap();
    let saved = f
        .store
        .install_package_skills(&online.id, "com.acme.pdf", "1.0.0", vec![package_entry("pdf-forms")], &tags, true)
        .unwrap();
    assert_eq!(saved[0].status, SkillStatus::Pending);
    assert_eq!(saved[0].source.package_id(), Some("com.acme.pdf"));
    assert!(f.store.active_skill_index(&online.id).unwrap().is_empty());
}

#[test]
fn deleting_the_last_skill_of_a_package_releases_what_this_device_installed() {
    let f = fixture();
    let bot = skilled_bot(&f);
    f.store
        .install_package_skills(
            &bot.id,
            "com.acme.pdf",
            "1.0.0",
            vec![package_entry("one"), package_entry("two")],
            &[],
            true,
        )
        .unwrap();
    let skills = f.store.skills(&bot.id).unwrap();
    f.store.delete_skill(&bot.id, &skills[0].id).unwrap();
    assert!(!f.store.release_package("com.acme.pdf").unwrap(), "another skill still uses it");
    f.store.delete_skill(&bot.id, &skills[1].id).unwrap();
    assert!(f.store.release_package("com.acme.pdf").unwrap());
    assert!(!f.store.release_package("com.acme.pdf").unwrap(), "released once");
}

#[test]
fn a_package_installed_before_is_never_released() {
    let f = fixture();
    let bot = skilled_bot(&f);
    let saved = f
        .store
        .install_package_skills(&bot.id, "com.acme.pdf", "1.0.0", vec![package_entry("one")], &[], false)
        .unwrap();
    f.store.delete_skill(&bot.id, &saved[0].id).unwrap();
    assert!(!f.store.release_package("com.acme.pdf").unwrap());
}
