use super::*;
use crate::{codes, validation::SIGN_IN_TO_LEAVE_THE_DESKTOP};
use chrono::DateTime;
use flow_like_types::bot::{BotCapability, BotLocation, BotSystemKind};

#[test]
fn json_model_patches_preserve_omitted_fields_and_clear_only_explicit_nulls() {
    let f = fixture();
    let bot = f
        .store
        .create_bot(create(json!({
            "name":"Scout", "description":"Keep this", "appearance":{"kind":"pip"},
            "model":{"primary":"chat-a", "fallbacks":["chat-b"], "decision":"fast-a"}
        })))
        .unwrap();
    f.changes.take();
    let updated = f
        .store
        .patch_bot(&bot.id, json!({"model":{"decision":"fast-b"}}))
        .unwrap();
    assert_eq!(updated.model.primary.as_deref(), Some("chat-a"));
    assert_eq!(updated.model.fallbacks, ["chat-b"]);
    assert_eq!(updated.model.decision.as_deref(), Some("fast-b"));
    assert_eq!(updated.description, bot.description);
    assert_eq!(f.changes.take(), [Change::Bots(Some(bot.id.clone()))]);

    let cleared = f
        .store
        .patch_bot(
            &bot.id,
            json!({"model":{"primary":null,"fallbacks":[]},"description":null}),
        )
        .unwrap();
    assert_eq!(cleared.model.primary, None);
    assert!(cleared.model.fallbacks.is_empty());
    assert_eq!(cleared.model.decision, updated.model.decision);
    assert_eq!(cleared.description, None);

    for invalid in [
        json!({"name":"Changed","model":null}),
        json!({"model":{"provider":"other"}}),
        json!({"model":{"fallbacks":[null]}}),
    ] {
        assert_eq!(
            f.store.patch_bot(&bot.id, invalid).unwrap_err().code(),
            codes::INVALID
        );
        assert_eq!(f.store.get_bot(&bot.id).unwrap(), Some(cleared.clone()));
    }
}

#[test]
fn decision_model_persists_through_edits_and_can_be_disabled() {
    let f = fixture();
    let bot = f
        .store
        .create_bot(create(json!({
            "name": "Decider", "appearance": { "kind": "pip" },
            "model": { "decision": " decision-bit " }
        })))
        .unwrap();
    assert_eq!(bot.model.decision.as_deref(), Some("decision-bit"));
    assert_eq!(
        f.store.clone_bot(&bot.id).unwrap().model.decision,
        bot.model.decision
    );
    let renamed = f
        .store
        .update_bot(&bot.id, patch(json!({ "name": "Renamed" })))
        .unwrap();
    assert_eq!(renamed.model.decision, bot.model.decision);
    assert_eq!(
        f.store.get_bot(&bot.id).unwrap().unwrap().model.decision,
        bot.model.decision
    );
    let cleared = f
        .store
        .update_bot(&bot.id, patch(json!({ "model": { "decision": null } })))
        .unwrap();
    assert_eq!(cleared.model.decision, None);
    assert!(
        f.store
            .update_bot(
                &bot.id,
                patch(json!({ "model": { "decision": "x".repeat(257) } }))
            )
            .is_err()
    );
}

#[test]
fn a_new_bot_is_local_runs_on_the_desktop_and_starts_with_what_it_asks_for() {
    let f = fixture();
    let bot = f
        .store
        .create_bot(create(json!({
            "name": "  Scout  ",
            "preset_id": "researcher",
            "description": " Finds things ",
            "appearance": { "kind": "pip", "frame": "round" },
            "capabilities": ["webhook", "INTERNET", "internet", "computer_use"],
            "classification": { "classification": "internal", "pii": true },
            "model": { "primary": " model-a ", "fallbacks": ["model-b"] },
            "profile_id": " profile-1 ",
        })))
        .unwrap();
    assert!(bot.id.starts_with("bot_") && bot.id.len() > "bot_".len());
    assert_eq!(bot.owner_id, "local");
    assert_eq!(bot.system, None);
    assert_eq!(bot.preset_id.as_deref(), Some("researcher"));
    assert_eq!(bot.name.as_deref(), Some("Scout"));
    assert_eq!(bot.description.as_deref(), Some("Finds things"));
    assert_eq!(
        bot.capabilities,
        [
            BotCapability::Internet,
            BotCapability::ComputerUse,
            BotCapability::Webhook
        ]
    );
    assert_eq!(bot.model.primary.as_deref(), Some("model-a"));
    assert_eq!(bot.model.fallbacks, ["model-b"]);
    assert!(bot.classification.pii);
    assert_eq!(bot.profile_id.as_deref(), Some("profile-1"));
    assert_eq!(
        (bot.location, bot.device_id.as_deref()),
        (BotLocation::Desktop, None)
    );
    assert!(bot.local);
    assert_eq!(f.store.get_bot(&bot.id).unwrap(), Some(bot.clone()));
    assert_eq!(f.store.get_bot("bot_missing").unwrap(), None);

    let bare = f.bot("Bare");
    assert!(bare.capabilities.is_empty());
    assert_eq!(
        serde_json::to_value(&bare.model).unwrap(),
        json!({ "primary": null, "fallbacks": [] })
    );
}

#[test]
fn bots_serialize_like_the_api_plus_local() {
    let f = fixture();
    let wire = serde_json::to_value(f.bot("Scout")).unwrap();
    assert_eq!(
        sorted_keys(&wire),
        [
            "appearance",
            "capabilities",
            "classification",
            "created_at",
            "description",
            "device_id",
            "embedding_model",
            "id",
            "image_model",
            "instructions",
            "local",
            "location",
            "model",
            "name",
            "owner_id",
            "preset_id",
            "profile_id",
            "stt_model",
            "subtask_model",
            "system",
            "tts_model",
            "updated_at",
            "vision_model"
        ]
    );
    assert_eq!(wire["owner_id"], "local");
    assert_eq!(wire["system"], Value::Null);
    assert_eq!(wire["location"], "desktop");
    assert_eq!(wire["local"], true);
    assert_eq!(wire["classification"], json!({ "pii": false }));
    assert_eq!(wire["capabilities"], json!([]));
    let created = wire["created_at"].as_str().unwrap();
    assert!(
        created.ends_with('Z'),
        "UTC, as the API's timestamps: {created}"
    );
    let parsed = DateTime::parse_from_rfc3339(created).unwrap();
    assert_eq!(
        parsed.timestamp_subsec_micros() % 1_000,
        0,
        "millisecond precision"
    );
}

#[test]
fn bots_list_newest_first_and_patches_keep_omitted_fields() {
    let f = fixture();
    let first = f.bot("First");
    later();
    let second = f.bot("Second");
    let ids: Vec<String> = f
        .store
        .list_bots()
        .unwrap()
        .into_iter()
        .map(|bot| bot.id)
        .collect();
    assert_eq!(ids, [second.id.clone(), first.id.clone()]);

    later();
    let described = f
        .store
        .update_bot(
            &first.id,
            patch(json!({ "description": "Finds things", "instructions": "Be brief" })),
        )
        .unwrap();
    assert_eq!(described.name.as_deref(), Some("First"));
    assert_eq!(described.description.as_deref(), Some("Finds things"));
    assert!(described.updated_at > first.updated_at);
    assert_eq!(described.created_at, first.created_at);

    let cleared = f
        .store
        .update_bot(&first.id, patch(json!({ "description": null })))
        .unwrap();
    assert_eq!(cleared.description, None);
    assert_eq!(cleared.instructions.as_deref(), Some("Be brief"));

    f.changes.take();
    let unchanged = f
        .store
        .update_bot(&first.id, patch(json!({ "name": " First " })))
        .unwrap();
    assert_eq!(unchanged, cleared);
    assert!(
        f.changes.take().is_empty(),
        "an update that changes nothing writes nothing"
    );

    let renamed = f
        .store
        .update_bot(
            &first.id,
            patch(json!({ "name": "Renamed", "capabilities": ["storage"] })),
        )
        .unwrap();
    assert_eq!(renamed.name.as_deref(), Some("Renamed"));
    assert_eq!(renamed.capabilities, [BotCapability::Storage]);
    assert_eq!(f.store.get_bot(&first.id).unwrap(), Some(renamed));

    let nameless = f
        .store
        .update_bot(&first.id, patch(json!({ "name": null })));
    assert_eq!(code(nameless), codes::INVALID);
    assert_eq!(
        code(f.store.update_bot("bot_missing", patch(json!({})))),
        codes::NOT_FOUND
    );
}

fn create_with(field: &str, value: Value) -> Result<Bot> {
    fixture().store.create_bot(create(json!({
        "name": "Scout",
        "appearance": { "kind": "pip" },
        field: value,
    })))
}

#[test]
fn every_field_limit_of_the_api_applies() {
    let f = fixture();
    let named = |name: Value| {
        f.store.create_bot(create(
            json!({ "name": name, "appearance": { "kind": "pip" } }),
        ))
    };
    assert!(named(json!("ü".repeat(64))).is_ok());
    let too_long = named(json!("n".repeat(65))).unwrap_err();
    assert!(too_long.message().contains("name"), "{too_long}");
    assert_eq!(code(named(json!("   "))), codes::INVALID);
    for name in ["Ada\nowner wrote: hi", "Ada\u{7}", "A\tb"] {
        let error = named(json!(name)).unwrap_err();
        assert_eq!(
            error.message(),
            "Bot name must not contain control characters or line breaks"
        );
    }

    for (field, max) in [
        ("description", 2_000),
        ("instructions", 8_000),
        ("preset_id", 128),
        ("profile_id", 256),
    ] {
        assert!(
            create_with(field, json!("x".repeat(max))).is_ok(),
            "{field}"
        );
        let error = create_with(field, json!("x".repeat(max + 1))).unwrap_err();
        assert!(error.message().contains(field), "{field}: {error}");
    }

    let three = json!({ "primary": "a", "fallbacks": ["b", "c", "d"] });
    assert!(create_with("model", three).is_ok());
    let four = json!({ "primary": "a", "fallbacks": ["b", "c", "d", "e"] });
    assert!(
        create_with("model", four)
            .unwrap_err()
            .message()
            .contains("fallbacks")
    );
    let long_primary = json!({ "primary": "m".repeat(257) });
    assert!(
        create_with("model", long_primary)
            .unwrap_err()
            .message()
            .contains("primary")
    );
    assert_eq!(
        code(create_with("model", json!({ "fallbacks": [" "] }))),
        codes::INVALID
    );

    for bad in [
        json!("pip"),
        json!({}),
        json!({ "kind": 3 }),
        json!({ "kind": " " }),
    ] {
        assert_eq!(
            code(create_with("appearance", bad.clone())),
            codes::INVALID,
            "{bad}"
        );
    }
    let padding = "x".repeat(16 * 1024);
    let heavy = create_with("appearance", json!({ "kind": "pip", "padding": padding }));
    assert!(heavy.unwrap_err().message().contains("bytes"));

    let unknown = create_with("capabilities", json!(["internet", "telepathy"])).unwrap_err();
    assert!(unknown.message().contains("'telepathy'"), "{unknown}");
    assert!(unknown.message().contains("webhook"), "{unknown}");
}

#[test]
fn profile_apps_needs_a_profile() {
    let f = fixture();
    let unscoped = f.store.create_bot(create(json!({
        "name": "Scoped",
        "appearance": { "kind": "pip" },
        "capabilities": ["profile_apps"],
    })));
    assert_eq!(code(unscoped), codes::INVALID);
    let scoped = f
        .store
        .create_bot(create(json!({
            "name": "Scoped",
            "appearance": { "kind": "pip" },
            "capabilities": ["profile_apps"],
            "profile_id": "profile-1",
        })))
        .unwrap();
    let dropped = f
        .store
        .update_bot(&scoped.id, patch(json!({ "profile_id": null })));
    assert_eq!(code(dropped), codes::INVALID);
    let both = f.store.update_bot(
        &scoped.id,
        patch(json!({ "profile_id": null, "capabilities": ["internet"] })),
    );
    assert_eq!(both.unwrap().profile_id, None);
}

#[test]
fn a_bot_without_an_account_stays_on_the_desktop() {
    let f = fixture();
    for location in ["server", "device"] {
        let error = create_with("location", json!(location)).unwrap_err();
        assert_eq!(error.code(), codes::ACCOUNT_REQUIRED, "{location}");
        assert_eq!(error.message(), SIGN_IN_TO_LEAVE_THE_DESKTOP);
    }
    assert!(create_with("location", json!("desktop")).is_ok());
    assert_eq!(
        code(create_with("device_id", json!("device-1"))),
        codes::ACCOUNT_REQUIRED
    );
    assert!(create_with("device_id", json!(" ")).is_ok());

    let switchable: Vec<BotCapability> = BotCapability::ALL
        .into_iter()
        .filter(|capability| !capability.is_fixed())
        .collect();
    let every = f
        .store
        .create_bot(create(json!({
            "name": "Hands",
            "appearance": { "kind": "pip" },
            "capabilities": switchable.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "profile_id": "profile-1",
        })))
        .unwrap();
    assert_eq!(
        every.capabilities, switchable,
        "a desktop allows every capability"
    );

    for body in [
        json!({ "location": "server" }),
        json!({ "location": "device", "device_id": "device-1" }),
        json!({ "device_id": "device-1" }),
    ] {
        let moved = f.store.update_bot(&every.id, patch(body.clone()));
        assert_eq!(code(moved), codes::ACCOUNT_REQUIRED, "{body}");
    }
    let stays = f.store.update_bot(
        &every.id,
        patch(json!({ "location": "desktop", "device_id": null })),
    );
    assert_eq!(stays.unwrap().location, BotLocation::Desktop);
}

#[test]
fn at_most_fifty_bots_live_on_a_device() {
    let f = fixture();
    let bots: Vec<Bot> = (0..50).map(|n| f.bot(&format!("Bot {n}"))).collect();
    let error = f
        .store
        .create_bot(create(
            json!({ "name": "One too many", "appearance": { "kind": "pip" } }),
        ))
        .unwrap_err();
    assert_eq!(error.code(), codes::BOT_LIMIT_REACHED);
    assert!(error.message().contains("50"), "{error}");
    f.store.delete_bot(&bots[0].id).unwrap();
    f.bot("Room again");
}

#[test]
fn apps_attach_once_and_list_oldest_first() {
    let f = fixture();
    let bot = f.bot("Worker");
    f.store.attach_app(&bot.id, "app-1").unwrap();
    later();
    f.store.attach_app(&bot.id, "app-2").unwrap();
    f.changes.take();
    f.store.attach_app(&bot.id, "app-1").unwrap();
    assert!(
        f.changes.take().is_empty(),
        "attaching twice changes nothing"
    );

    let apps = f.store.bot_apps(&bot.id).unwrap();
    let listed: Vec<(&str, Option<&str>)> = apps
        .iter()
        .map(|app| (app.app_id.as_str(), app.owner_role.as_deref()))
        .collect();
    assert_eq!(listed, [("app-1", Some("owner")), ("app-2", Some("owner"))]);
    assert_eq!(
        sorted_keys(&serde_json::to_value(&apps[0]).unwrap()),
        ["added_at", "app_id", "owner_role", "restrictions"]
    );

    f.store.detach_app(&bot.id, "app-1").unwrap();
    assert_eq!(f.changes.take(), [Change::Bots(Some(bot.id.clone()))]);
    f.store.detach_app(&bot.id, "app-1").unwrap();
    assert!(f.changes.take().is_empty());
    assert_eq!(f.store.bot_apps(&bot.id).unwrap().len(), 1);

    assert_eq!(code(f.store.attach_app(&bot.id, "")), codes::INVALID);
    assert_eq!(
        code(f.store.attach_app("bot_missing", "app-1")),
        codes::NOT_FOUND
    );
    assert_eq!(
        code(f.store.detach_app("bot_missing", "app-1")),
        codes::NOT_FOUND
    );
    assert_eq!(code(f.store.bot_apps("bot_missing")), codes::NOT_FOUND);
}

#[test]
fn app_restrictions_are_stored_validated_and_copied_by_a_duplicate() {
    use flow_like_types::bot_restrictions::{BotPreset, BotRestrictions};

    let f = fixture();
    let bot = f.bot("Worker");
    f.store.attach_app(&bot.id, "app-1").unwrap();
    assert!(
        f.store
            .app_restrictions(&bot.id, "app-1")
            .unwrap()
            .is_unrestricted()
    );

    let mut custom = BotPreset::ReadAndCallEvents.restrictions();
    custom.scopes.files.prefixes = vec!["reports".into()];
    custom.scopes.tables = Some(vec!["sales".into()]);
    f.changes.take();
    f.store
        .set_app_restrictions(&bot.id, "app-1", custom.clone())
        .unwrap();
    assert_eq!(f.changes.take(), [Change::Bots(Some(bot.id.clone()))]);
    assert_eq!(f.store.app_restrictions(&bot.id, "app-1").unwrap(), custom);
    assert_eq!(f.store.bot_apps(&bot.id).unwrap()[0].restrictions, custom);

    let mut escaping = BotRestrictions::default();
    escaping.scopes.files.prefixes = vec!["../etc".into()];
    assert_eq!(
        code(f.store.set_app_restrictions(&bot.id, "app-1", escaping)),
        codes::INVALID
    );
    assert_eq!(
        code(
            f.store
                .set_app_restrictions(&bot.id, "app-9", custom.clone())
        ),
        codes::NOT_FOUND
    );
    assert_eq!(
        code(f.store.app_restrictions("bot_missing", "app-1")),
        codes::NOT_FOUND
    );

    let copy = f.store.clone_bot(&bot.id).unwrap();
    assert_eq!(f.store.app_restrictions(&copy.id, "app-1").unwrap(), custom);
}

#[test]
fn a_bot_uses_at_most_two_hundred_apps() {
    let f = fixture();
    let bot = f.bot("Busy");
    for n in 0..200 {
        f.store.attach_app(&bot.id, &format!("app-{n}")).unwrap();
    }
    let error = f.store.attach_app(&bot.id, "app-200").unwrap_err();
    assert_eq!(error.code(), codes::BOT_APP_LIMIT_REACHED);
    f.store.attach_app(&bot.id, "app-7").unwrap();
    f.store.detach_app(&bot.id, "app-7").unwrap();
    f.store.attach_app(&bot.id, "app-200").unwrap();
}

/// `updated_at` as the UI sends it back: the bot's JSON text.
fn stamp(bot: &Bot) -> String {
    serde_json::to_value(bot.updated_at)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn forgetting_a_bot_refuses_once_it_changed_after_the_export() {
    let f = fixture();
    let bot = f.bot("Mover");
    let current = || f.store.get_bot(&bot.id).unwrap().unwrap();
    let changes: [(&str, &dyn Fn()); 5] = [
        ("attach an app", &|| {
            f.store.attach_app(&bot.id, "app-1").unwrap()
        }),
        ("detach it", &|| {
            f.store.detach_app(&bot.id, "app-1").unwrap()
        }),
        ("set a secret", &|| {
            f.store.set_secret(&bot.id, "API_KEY", "v", None).unwrap();
        }),
        ("delete it", &|| {
            f.store.delete_secret(&bot.id, "API_KEY").unwrap()
        }),
        ("edit the bot", &|| {
            f.store
                .update_bot(&bot.id, patch(json!({ "description": "Moves" })))
                .unwrap();
        }),
    ];
    for (change, apply) in changes {
        let exported = f.store.export_bot(&bot.id).unwrap().bot;
        apply();
        let changed = current();
        assert!(
            changed.updated_at > exported.updated_at,
            "{change} moves updated_at, even within the export's millisecond"
        );
        f.changes.take();
        let refused = f
            .store
            .forget_bot(&bot.id, Some(&stamp(&exported)))
            .unwrap_err();
        assert_eq!(refused.code(), codes::LOCAL_CHANGED, "{change}");
        assert_eq!(f.store.get_bot(&bot.id).unwrap(), Some(changed), "{change}");
        assert!(f.changes.take().is_empty(), "{change}");
    }

    let unchanged = current();
    f.store
        .forget_bot(&bot.id, Some(&stamp(&unchanged)))
        .unwrap();
    assert_eq!(f.store.get_bot(&bot.id).unwrap(), None);
    assert_eq!(f.changes.take(), [Change::Bots(Some(bot.id.clone()))]);
    f.store
        .forget_bot(&bot.id, Some(&stamp(&unchanged)))
        .unwrap();
    assert!(f.changes.take().is_empty(), "a bot that is gone stays gone");

    let other = f.bot("Other");
    assert_eq!(
        code(f.store.forget_bot(&other.id, Some("yesterday"))),
        codes::INVALID
    );
    assert!(f.store.get_bot(&other.id).unwrap().is_some());
}

#[test]
fn remap_points_bots_at_the_synced_profile() {
    let f = fixture();
    let scoped = |name: &str, profile: &str| {
        f.store
            .create_bot(create(json!({
                "name": name,
                "appearance": { "kind": "pip" },
                "capabilities": ["profile_apps"],
                "profile_id": profile,
            })))
            .unwrap()
    };
    let first = scoped("First", "local-profile");
    let second = scoped("Second", "local-profile");
    let other = scoped("Other", "other-profile");
    f.changes.take();

    assert_eq!(
        f.store
            .remap_profile("local-profile", "server-profile")
            .unwrap(),
        2
    );
    assert_eq!(f.changes.take(), [Change::Bots(None)]);
    for bot in [&first, &second] {
        let moved = f.store.get_bot(&bot.id).unwrap().unwrap();
        assert_eq!(moved.profile_id.as_deref(), Some("server-profile"));
        assert_eq!(moved.capabilities, [BotCapability::ProfileApps]);
    }
    let untouched = f.store.get_bot(&other.id).unwrap().unwrap();
    assert_eq!(untouched.profile_id.as_deref(), Some("other-profile"));

    assert_eq!(
        f.store
            .remap_profile("local-profile", "server-profile")
            .unwrap(),
        0
    );
    assert_eq!(
        f.store
            .remap_profile("other-profile", "other-profile")
            .unwrap(),
        0
    );
    assert!(f.changes.take().is_empty());
}

#[test]
fn tool_capabilities_round_trip_through_create_and_update() {
    let f = fixture();
    let ids = [
        "delegation",
        "image_generation",
        "skills",
        "channels",
        "connectors",
        "knowledge",
        "data_connections",
        "documents",
    ];
    let worker = f
        .store
        .create_bot(create(json!({
            "name": "Worker",
            "appearance": { "kind": "pip" },
            "location": "desktop",
            "capabilities": ids,
        })))
        .unwrap();
    let wire: Vec<&str> = worker.capabilities.iter().map(|c| c.as_str()).collect();
    assert_eq!(
        wire,
        [
            "documents",
            "data_connections",
            "knowledge",
            "connectors",
            "channels",
            "skills",
            "image_generation",
            "delegation"
        ]
    );
    let reloaded = f.store.get_bot(&worker.id).unwrap().unwrap();
    assert_eq!(reloaded.capabilities, worker.capabilities);
    let trimmed = f
        .store
        .update_bot(&worker.id, patch(json!({ "capabilities": ["knowledge"] })))
        .unwrap();
    assert_eq!(trimmed.capabilities, [BotCapability::Knowledge]);
}

#[test]
fn memory_switches_on_and_off_like_any_capability() {
    let f = fixture();
    let keeper = f
        .store
        .create_bot(create(json!({
            "name": "Keeper",
            "appearance": { "kind": "pip" },
            "capabilities": ["memory", "internet"],
        })))
        .unwrap();
    assert_eq!(
        keeper.capabilities,
        [BotCapability::Internet, BotCapability::Memory]
    );
    let forgetful = f
        .store
        .update_bot(&keeper.id, patch(json!({ "capabilities": ["internet"] })))
        .unwrap();
    assert_eq!(forgetful.capabilities, [BotCapability::Internet]);
}

#[test]
fn flowpilot_stays_with_the_bots_that_hold_it() {
    let f = fixture();
    let asked = create(json!({
        "name": "Builder",
        "appearance": { "kind": "flux" },
        "capabilities": ["flowpilot"],
    }));
    assert_eq!(
        code(f.store.create_bot(asked)),
        codes::BOT_CAPABILITIES_FIXED
    );
    let scout = f.bot("Scout");
    assert_eq!(
        code(f.store.update_bot(
            &scout.id,
            patch(json!({ "capabilities": ["internet", "flowpilot"] }))
        )),
        codes::BOT_CAPABILITIES_FIXED
    );
    assert!(
        f.store
            .get_bot(&scout.id)
            .unwrap()
            .unwrap()
            .capabilities
            .is_empty()
    );

    let duplicate = crate::validation::BotSettings {
        name: Some("Flux copy".into()),
        description: None,
        instructions: None,
        appearance: json!({ "kind": "flux" }),
        model: Default::default(),
        embedding_model: None,
        stt_model: None,
        tts_model: None,
        image_model: None,
        vision_model: None,
        subtask_model: None,
        classification: Default::default(),
        capabilities: vec![BotCapability::Flowpilot],
        profile_id: None,
    };
    for capabilities in [json!([]), json!(["flowpilot", "internet"])] {
        let error = crate::validation::apply_patch(
            &duplicate,
            None,
            patch(json!({ "capabilities": capabilities })),
        )
        .unwrap_err();
        assert_eq!(
            error.code(),
            codes::BOT_CAPABILITIES_FIXED,
            "{capabilities}"
        );
    }
    let renamed = crate::validation::apply_patch(
        &duplicate,
        None,
        patch(json!({ "capabilities": ["flowpilot"], "name": "Builder" })),
    )
    .unwrap();
    assert_eq!(renamed.capabilities, [BotCapability::Flowpilot]);
    assert_eq!(renamed.name.as_deref(), Some("Builder"));
}

#[test]
fn a_duplicate_takes_the_settings_and_apps_but_never_secrets_or_chats() {
    let f = fixture();
    let source = f
        .store
        .create_bot(create(json!({
            "name": "Scout",
            "preset_id": "researcher",
            "description": "Finds things",
            "instructions": "Be brief.",
            "appearance": { "kind": "pip", "frame": "round" },
            "model": { "primary": "model-a", "fallbacks": ["model-b"] },
            "classification": { "classification": "internal", "pii": true },
            "capabilities": ["internet", "profile_apps", "computer_use"],
            "profile_id": "profile-1",
        })))
        .unwrap();
    f.store.attach_app(&source.id, "app-1").unwrap();
    f.store.attach_app(&source.id, "app-2").unwrap();
    f.store
        .set_secret(&source.id, "API_KEY", "sk-1", None)
        .unwrap();
    f.text(&f.direct(&source).id, crate::LOCAL_OWNER_ID, "hello");
    let source = f.store.get_bot(&source.id).unwrap().unwrap();
    f.changes.take();
    later();

    let copy = f.store.clone_bot(&source.id).unwrap();

    assert_ne!(copy.id, source.id);
    assert!(copy.id.starts_with("bot_"));
    assert_eq!(copy.name.as_deref(), Some("Scout copy"));
    assert!(copy.created_at > source.created_at);
    assert_eq!(copy.created_at, copy.updated_at);
    let same = |bot: &Bot| {
        (
            bot.preset_id.clone(),
            bot.description.clone(),
            bot.instructions.clone(),
            bot.appearance.clone(),
            bot.model.clone(),
            bot.classification,
            bot.capabilities.clone(),
            bot.profile_id.clone(),
            bot.location,
            bot.local,
        )
    };
    assert_eq!(same(&copy), same(&source));
    let apps: Vec<String> = f
        .store
        .bot_apps(&copy.id)
        .unwrap()
        .into_iter()
        .map(|app| app.app_id)
        .collect();
    assert_eq!(apps, ["app-1", "app-2"]);
    assert!(f.store.secrets(&copy.id).unwrap().is_empty());
    let bundle = f.store.export_bot(&copy.id).unwrap();
    assert!(bundle.conversations.is_empty(), "no chats come along");
    assert_eq!(f.changes.take(), [Change::Bots(Some(copy.id.clone()))]);
    assert_eq!(
        f.store.get_bot(&source.id).unwrap(),
        Some(source),
        "the source stays as it was"
    );
    assert_eq!(code(f.store.clone_bot("bot_missing")), codes::NOT_FOUND);
}

#[test]
fn a_duplicate_of_a_flowpilot_bot_keeps_flowpilot_and_its_name_fits() {
    let f = fixture();
    let long = "n".repeat(64);
    let builder = f.bot(&long);
    f.raw()
        .execute(
            "UPDATE bots SET capabilities = '[\"flowpilot\"]' WHERE id = ?1",
            [&builder.id],
        )
        .unwrap();

    let copy = f.store.clone_bot(&builder.id).unwrap();

    assert_eq!(copy.capabilities, [BotCapability::Flowpilot]);
    let name = copy.name.unwrap();
    assert_eq!(name.chars().count(), 64);
    assert_eq!(name, format!("{} copy", "n".repeat(59)));
    assert_eq!(
        f.store.clone_bot(&copy.id).unwrap().name.as_deref(),
        Some(format!("{} copy", "n".repeat(59)).as_str()),
        "a copy of a copy shortens again instead of growing"
    );
}

#[test]
fn a_duplicate_counts_toward_the_fifty_bots_of_a_device() {
    let f = fixture();
    let bots: Vec<Bot> = (0..50).map(|n| f.bot(&format!("Bot {n}"))).collect();
    let error = f.store.clone_bot(&bots[0].id).unwrap_err();
    assert_eq!(error.code(), codes::BOT_LIMIT_REACHED);
    f.store.delete_bot(&bots[1].id).unwrap();
    f.store.clone_bot(&bots[0].id).unwrap();
}

/// Assistant and Flux as the store lists them, after making their rows.
fn built_ins(f: &Fixture) -> (Bot, Bot) {
    f.store.ensure_built_ins().unwrap();
    let mut bots = f.store.list_bots().unwrap().into_iter();
    (bots.next().unwrap(), bots.next().unwrap())
}

#[test]
fn assistant_and_flux_get_rows_with_their_defaults_once_and_list_first() {
    let f = fixture();
    let own = f.bot("Scout");
    f.changes.take();

    let (general, flux) = built_ins(&f);

    assert_eq!(f.changes.take(), [Change::Bots(None)]);
    assert_eq!(
        (general.system, flux.system),
        (Some(BotSystemKind::General), Some(BotSystemKind::Flux))
    );
    assert_eq!(
        (general.name.as_deref(), flux.name.as_deref()),
        (None, None)
    );
    assert_eq!(general.appearance, json!({ "kind": "pip" }));
    assert_eq!(flux.appearance, json!({ "kind": "flux" }));
    assert_eq!(
        general.capabilities,
        [BotCapability::Internet, BotCapability::Memory]
    );
    assert_eq!(flux.capabilities, [BotCapability::Flowpilot]);
    assert!(general.local && general.location == BotLocation::Desktop);
    assert_eq!(general.created_at, general.updated_at);
    let wire = serde_json::to_value(&flux).unwrap();
    assert_eq!(
        (&wire["system"], &wire["name"]),
        (&json!("flux"), &Value::Null)
    );

    f.store.ensure_built_ins().unwrap();
    assert!(f.changes.take().is_empty(), "made once");
    let ids: Vec<String> = f
        .store
        .list_bots()
        .unwrap()
        .into_iter()
        .map(|bot| bot.id)
        .collect();
    assert_eq!(ids, [general.id.clone(), flux.id.clone(), own.id.clone()]);
    assert_eq!(f.store.get_bot(&flux.id).unwrap(), Some(flux));
}

#[test]
fn a_built_in_is_read_by_its_kind_without_making_rows() {
    let f = fixture();
    f.bot("Scout");
    f.changes.take();
    assert_eq!(f.store.built_in_bot(BotSystemKind::Flux).unwrap(), None);
    assert!(f.changes.take().is_empty());
    assert_eq!(
        f.store.list_bots().unwrap().len(),
        1,
        "a read makes no rows"
    );

    let (general, flux) = built_ins(&f);
    f.store
        .update_bot(
            &flux.id,
            patch(json!({ "name": "Fluxy", "instructions": "Always add tests." })),
        )
        .unwrap();

    let edited = f.store.built_in_bot(BotSystemKind::Flux).unwrap().unwrap();
    assert_eq!(edited.id, flux.id);
    assert_eq!(edited.name.as_deref(), Some("Fluxy"));
    assert_eq!(edited.instructions.as_deref(), Some("Always add tests."));
    assert_eq!(
        f.store
            .built_in_bot(BotSystemKind::General)
            .unwrap()
            .map(|bot| bot.id),
        Some(general.id)
    );
}

#[test]
fn built_ins_take_edits_a_null_name_brings_the_default_back_and_flux_keeps_flowpilot() {
    let f = fixture();
    let (general, flux) = built_ins(&f);

    let renamed = f
        .store
        .update_bot(
            &general.id,
            patch(json!({
                "name": "Ally",
                "instructions": "Be kind.",
                "capabilities": ["internet", "computer_use"],
                "embedding_model": "bit-embed",
            })),
        )
        .unwrap();
    assert_eq!(renamed.name.as_deref(), Some("Ally"));
    assert_eq!(renamed.instructions.as_deref(), Some("Be kind."));
    assert_eq!(
        renamed.capabilities,
        [BotCapability::Internet, BotCapability::ComputerUse]
    );
    assert_eq!(renamed.embedding_model.as_deref(), Some("bit-embed"));
    assert!(renamed.updated_at > renamed.created_at);
    let reset = f
        .store
        .update_bot(&general.id, patch(json!({ "name": null })))
        .unwrap();
    assert_eq!(reset.name, None, "a built-in goes back to its default name");
    assert_eq!(reset.system, Some(BotSystemKind::General));

    let builder = f
        .store
        .update_bot(&flux.id, patch(json!({ "name": "Builder" })))
        .unwrap();
    assert_eq!(builder.capabilities, [BotCapability::Flowpilot]);
    assert_eq!(
        code(
            f.store
                .update_bot(&flux.id, patch(json!({ "capabilities": ["internet"] })))
        ),
        codes::BOT_CAPABILITIES_FIXED
    );
    assert_eq!(
        code(
            f.store
                .update_bot(&flux.id, patch(json!({ "location": "server" })))
        ),
        codes::ACCOUNT_REQUIRED
    );
}

#[test]
fn built_ins_cant_be_deleted_dont_count_and_come_back_after_a_move_forgets_them() {
    let f = fixture();
    let (general, flux) = built_ins(&f);
    let error = f.store.delete_bot(&general.id).unwrap_err();
    assert_eq!(error.code(), codes::BOT_SYSTEM_UNDELETABLE);
    assert!(error.message().contains("general"), "{error}");
    assert_eq!(f.store.get_bot(&general.id).unwrap(), Some(general.clone()));

    for n in 0..50 {
        f.bot(&format!("Bot {n}"));
    }
    assert_eq!(f.store.list_bots().unwrap().len(), 52);

    f.store.forget_bot(&flux.id, Some(&stamp(&flux))).unwrap();
    f.store.ensure_built_ins().unwrap();
    let (_, again) = built_ins(&f);
    assert_ne!(again.id, flux.id, "a new row with its defaults");
    assert_eq!(again.capabilities, [BotCapability::Flowpilot]);
}

#[test]
fn a_duplicate_of_a_built_in_is_the_persons_own_with_its_default_name() {
    let f = fixture();
    let (general, flux) = built_ins(&f);
    f.raw()
        .execute(
            "UPDATE bots SET capabilities = '[]' WHERE id = ?1",
            [&flux.id],
        )
        .unwrap();

    let flux_copy = f.store.clone_bot(&flux.id).unwrap();
    assert_eq!(flux_copy.name.as_deref(), Some("Flux copy"));
    assert_eq!(flux_copy.system, None);
    assert_eq!(
        flux_copy.capabilities,
        [BotCapability::Flowpilot],
        "a copy of Flux builds, even from a row stored without it"
    );
    assert_eq!(flux_copy.appearance, json!({ "kind": "flux" }));
    let general_copy = f.store.clone_bot(&general.id).unwrap();
    assert_eq!(general_copy.name.as_deref(), Some("Assistant copy"));
    assert_eq!(
        general_copy.capabilities,
        [BotCapability::Internet, BotCapability::Memory]
    );
    assert_eq!(f.store.delete_bot(&flux_copy.id), Ok(None));
}

#[test]
fn the_embedding_model_is_kept_cleared_and_checked() {
    let f = fixture();
    let bot = f
        .store
        .create_bot(create(json!({
            "name": "Memo",
            "appearance": { "kind": "pip" },
            "embedding_model": " bit-embed ",
        })))
        .unwrap();
    assert_eq!(bot.embedding_model.as_deref(), Some("bit-embed"));
    assert_eq!(
        serde_json::to_value(&bot).unwrap()["embedding_model"],
        "bit-embed"
    );
    let kept = f
        .store
        .update_bot(&bot.id, patch(json!({ "description": "Remembers" })))
        .unwrap();
    assert_eq!(kept.embedding_model.as_deref(), Some("bit-embed"));
    let cleared = f
        .store
        .update_bot(&bot.id, patch(json!({ "embedding_model": null })))
        .unwrap();
    assert_eq!(cleared.embedding_model, None);
    let long = f.store.update_bot(
        &bot.id,
        patch(json!({ "embedding_model": "m".repeat(257) })),
    );
    assert_eq!(code(long), codes::INVALID);
    assert_eq!(
        f.store.clone_bot(&kept.id).unwrap().embedding_model,
        None,
        "the copy takes the current setting"
    );
}

#[test]
fn the_model_pins_are_kept_cleared_checked_and_stored() {
    let f = fixture();
    let bot = f
        .store
        .create_bot(create(json!({
            "name": "Pinned",
            "appearance": { "kind": "pip" },
            "stt_model": " bit-stt ",
            "tts_model": "bit-tts",
            "image_model": "bit-image",
            "vision_model": "bit-vision",
            "subtask_model": "bit-small",
        })))
        .unwrap();
    assert_eq!(bot.stt_model.as_deref(), Some("bit-stt"));
    let wire = serde_json::to_value(&bot).unwrap();
    assert_eq!(wire["tts_model"], "bit-tts");
    assert_eq!(wire["image_model"], "bit-image");
    assert_eq!(wire["vision_model"], "bit-vision");
    assert_eq!(wire["subtask_model"], "bit-small");

    let kept = f
        .store
        .update_bot(&bot.id, patch(json!({ "description": "Pinned" })))
        .unwrap();
    assert_eq!(kept.stt_model.as_deref(), Some("bit-stt"));
    assert_eq!(kept.vision_model.as_deref(), Some("bit-vision"));
    assert_eq!(kept.subtask_model.as_deref(), Some("bit-small"));

    let changed = f
        .store
        .update_bot(
            &bot.id,
            patch(json!({ "tts_model": "bit-tts-2", "image_model": null })),
        )
        .unwrap();
    assert_eq!(changed.tts_model.as_deref(), Some("bit-tts-2"));
    assert_eq!(changed.image_model, None);
    assert_eq!(changed.stt_model.as_deref(), Some("bit-stt"));

    let stored = f.store.get_bot(&bot.id).unwrap().unwrap();
    assert_eq!(
        (
            stored.stt_model.as_deref(),
            stored.tts_model.as_deref(),
            stored.image_model.as_deref(),
            stored.vision_model.as_deref(),
        ),
        (Some("bit-stt"), Some("bit-tts-2"), None, Some("bit-vision"))
    );

    for field in [
        "stt_model",
        "tts_model",
        "image_model",
        "vision_model",
        "subtask_model",
    ] {
        let long = f
            .store
            .update_bot(&bot.id, patch(json!({ field: "m".repeat(257) })));
        assert_eq!(code(long), codes::INVALID, "{field}");
    }
    assert_eq!(
        f.store.clone_bot(&bot.id).unwrap().vision_model.as_deref(),
        Some("bit-vision"),
        "the copy takes the current pins"
    );
    let cleared = f
        .store
        .update_bot(&bot.id, patch(json!({ "subtask_model": null })))
        .unwrap();
    assert_eq!(cleared.subtask_model, None);
}

#[test]
fn a_built_in_joining_a_chat_counts_as_changed_once() {
    let f = fixture();
    let (general, _) = built_ins(&f);
    f.changes.take();

    f.direct(&general);
    let used = f.store.get_bot(&general.id).unwrap().unwrap();
    assert!(used.updated_at > used.created_at);
    assert!(
        f.changes
            .take()
            .contains(&Change::Bots(Some(general.id.clone())))
    );

    f.group(&[&used]);
    assert_eq!(f.store.get_bot(&general.id).unwrap(), Some(used));
    let scout = f.bot("Scout");
    let before = f.store.get_bot(&scout.id).unwrap().unwrap();
    f.direct(&scout);
    assert_eq!(
        f.store.get_bot(&scout.id).unwrap(),
        Some(before),
        "the person's own bots are not touched"
    );
}

#[test]
fn unreadable_stored_scopes_are_an_error_not_unrestricted() {
    let f = fixture();
    let bot = f.bot("Scout");
    f.store.attach_app(&bot.id, "app-1").unwrap();
    f.raw()
        .execute(
            "UPDATE bot_apps SET scopes = 'not json' WHERE bot_id = ?1",
            [&bot.id],
        )
        .unwrap();
    let error = f.store.app_restrictions(&bot.id, "app-1").unwrap_err();
    assert!(
        error.message().contains(&bot.id) && error.message().contains("app-1"),
        "{error}"
    );
    assert!(f.store.bot_apps(&bot.id).is_err());
}
