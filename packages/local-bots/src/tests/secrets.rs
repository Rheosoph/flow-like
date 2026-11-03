use super::*;
use crate::{BotSecretEncryption, ExportedSecret, codes};

fn exported(name: &str, description: Option<&str>, readable: bool) -> ExportedSecret {
    ExportedSecret {
        name: name.into(),
        description: description.map(str::to_string),
        readable,
    }
}

/// The value the store opens for the upload.
fn opened_value(store: &LocalBotStore, bot_id: &str, name: &str) -> Option<String> {
    store
        .open_secrets(bot_id)
        .unwrap()
        .readable
        .into_iter()
        .find(|secret| secret.name == name)
        .map(|secret| secret.value.to_string())
}

fn stored_ciphertext(f: &Fixture, bot_id: &str, name: &str) -> Vec<u8> {
    f.raw()
        .query_row(
            "SELECT ciphertext FROM bot_secrets WHERE bot_id = ?1 AND name = ?2",
            [bot_id, name],
            |row| row.get(0),
        )
        .unwrap()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn secrets_round_trip_sealed_and_never_rest_in_plain_text() {
    let f = fixture();
    let bot = f.bot("Keeper");
    assert!(
        f.key.calls().is_empty(),
        "creating a bot never touches the keychain"
    );

    let info = f
        .store
        .set_secret(
            &bot.id,
            "API_KEY",
            "sk-secret-value",
            Some(Some(" OpenAI ".into())),
        )
        .unwrap();
    assert_eq!(info.name, "API_KEY");
    assert_eq!(info.encryption, BotSecretEncryption::Device);
    assert_eq!(info.description.as_deref(), Some("OpenAI"));
    assert_eq!(info.last_used_at, None);
    assert_eq!(f.key.calls(), [true]);

    let wire = serde_json::to_value(&info).unwrap();
    assert_eq!(
        sorted_keys(&wire),
        [
            "created_at",
            "description",
            "encryption",
            "last_used_at",
            "name",
            "updated_at"
        ]
    );
    assert_eq!(wire["encryption"], "device");

    let stored = stored_ciphertext(&f, &bot.id, "API_KEY");
    assert!(!contains(&stored, b"sk-secret-value"));
    assert_eq!(
        stored.len(),
        24 + "sk-secret-value".len() + 16,
        "nonce, value, tag"
    );
    let file = std::fs::read(&f.path).unwrap();
    assert!(!contains(&file, b"sk-secret-value"));

    assert_eq!(
        f.store.secrets(&bot.id).unwrap(),
        std::slice::from_ref(&info)
    );
    let bundle = f.store.export_bot(&bot.id).unwrap();
    assert_eq!(
        bundle.secrets,
        [exported("API_KEY", Some("OpenAI"), true)],
        "the export says the secret opens, without its value"
    );
    let wire = serde_json::to_value(&bundle).unwrap();
    assert_eq!(
        sorted_keys(&wire["secrets"][0]),
        ["description", "name", "readable"]
    );
    assert!(!wire.to_string().contains("sk-secret-value"));
    assert_eq!(
        f.key.calls(),
        [true, false],
        "exporting never creates a key"
    );

    let opened = f.store.open_secrets(&bot.id).unwrap();
    assert_eq!(opened.readable.len(), 1);
    assert_eq!(opened.readable[0].value.as_str(), "sk-secret-value");
    assert_eq!(opened.readable[0].description.as_deref(), Some("OpenAI"));
    assert!(opened.unreadable.is_empty());
    let debug = format!("{opened:?}");
    assert!(!debug.contains("sk-secret-value"), "{debug}");

    later();
    let replaced = f
        .store
        .set_secret(&bot.id, "API_KEY", "sk-rotated", None)
        .unwrap();
    assert_eq!(
        replaced.description.as_deref(),
        Some("OpenAI"),
        "omitted keeps"
    );
    assert_eq!(replaced.created_at, info.created_at);
    assert!(replaced.updated_at > info.updated_at);
    assert_ne!(stored_ciphertext(&f, &bot.id, "API_KEY"), stored);
    assert_eq!(
        opened_value(&f.store, &bot.id, "API_KEY").as_deref(),
        Some("sk-rotated")
    );

    let cleared = f
        .store
        .set_secret(&bot.id, "API_KEY", "sk-rotated", Some(None))
        .unwrap();
    assert_eq!(cleared.description, None, "null clears");
    let described = f
        .store
        .set_secret(&bot.id, "API_KEY", "sk-rotated", Some(Some("Named".into())))
        .unwrap();
    assert_eq!(described.description.as_deref(), Some("Named"));
    let blank = f
        .store
        .set_secret(&bot.id, "API_KEY", "sk-rotated", Some(Some("  ".into())))
        .unwrap();
    assert_eq!(blank.description, None, "an empty description clears");
}

#[test]
fn the_same_value_seals_differently_each_time() {
    let f = fixture();
    let bot = f.bot("Keeper");
    f.store.set_secret(&bot.id, "FIRST", "same", None).unwrap();
    f.store.set_secret(&bot.id, "SECOND", "same", None).unwrap();
    let first = stored_ciphertext(&f, &bot.id, "FIRST");
    let second = stored_ciphertext(&f, &bot.id, "SECOND");
    assert_ne!(first[..24], second[..24], "a fresh nonce per value");
}

#[test]
fn a_ciphertext_only_opens_in_the_row_it_was_sealed_for() {
    let f = fixture();
    let a = f.bot("A");
    let b = f.bot("B");
    f.store.set_secret(&a.id, "API_KEY", "sk-a", None).unwrap();
    f.store
        .set_secret(&a.id, "OTHER_KEY", "sk-other", None)
        .unwrap();
    f.store.set_secret(&b.id, "API_KEY", "sk-b", None).unwrap();
    let copy = |from: (&str, &str), to: (&str, &str)| {
        f.raw()
            .execute(
                "UPDATE bot_secrets SET ciphertext = (
                     SELECT ciphertext FROM bot_secrets WHERE bot_id = ?1 AND name = ?2)
                 WHERE bot_id = ?3 AND name = ?4",
                [from.0, from.1, to.0, to.1],
            )
            .unwrap()
    };

    assert_eq!(copy((&a.id, "API_KEY"), (&b.id, "API_KEY")), 1);
    assert_eq!(
        f.store.export_bot(&b.id).unwrap().secrets,
        [exported("API_KEY", None, false)]
    );
    let opened = f.store.open_secrets(&b.id).unwrap();
    assert!(opened.readable.is_empty());
    assert_eq!(opened.unreadable, ["API_KEY"]);

    assert_eq!(copy((&a.id, "API_KEY"), (&a.id, "OTHER_KEY")), 1);
    assert_eq!(
        f.store.export_bot(&a.id).unwrap().secrets,
        [
            exported("API_KEY", None, true),
            exported("OTHER_KEY", None, false)
        ],
        "one unreadable secret leaves the others readable"
    );
    let opened = f.store.open_secrets(&a.id).unwrap();
    assert_eq!(opened.readable.len(), 1);
    assert_eq!(opened.readable[0].value.as_str(), "sk-a");
    assert_eq!(opened.unreadable, ["OTHER_KEY"]);
}

#[test]
fn a_secret_opens_only_with_the_key_that_sealed_it() {
    let f = fixture();
    let bot = f.bot("Keeper");
    f.store
        .set_secret(&bot.id, "API_KEY", "sk-1", None)
        .unwrap();
    let unreadable = [exported("API_KEY", None, false)];

    let other = f.open_again(TestKey::holding([9; 32])).unwrap();
    assert_eq!(other.export_bot(&bot.id).unwrap().secrets, unreadable);
    assert_eq!(other.open_secrets(&bot.id).unwrap().unreadable, ["API_KEY"]);

    let key = Arc::new(TestKey::default());
    let keyless =
        LocalBotStore::open(&f.path, key.clone(), Arc::new(Recorder::new(&f.path))).unwrap();
    assert_eq!(
        keyless.export_bot(&bot.id).unwrap().secrets,
        unreadable,
        "without a key nothing opens"
    );
    assert_eq!(
        keyless.open_secrets(&bot.id).unwrap().unreadable,
        ["API_KEY"]
    );
    assert_eq!(
        key.calls(),
        [false, false],
        "a missing key is not made on read"
    );

    let locked = f.open_again(TestKey::refusing()).unwrap();
    let error = locked.export_bot(&bot.id).unwrap_err();
    assert_eq!(
        (error.code(), error.message()),
        (codes::SECRET_KEY, "the keychain is locked"),
        "a keychain that can't be asked now fails instead of calling secrets unreadable"
    );
    assert_eq!(code(locked.open_secrets(&bot.id)), codes::SECRET_KEY);

    let same = f.open_again(TestKey::holding([7; 32])).unwrap();
    assert_eq!(
        same.export_bot(&bot.id).unwrap().secrets,
        [exported("API_KEY", None, true)]
    );
    assert_eq!(
        opened_value(&same, &bot.id, "API_KEY").as_deref(),
        Some("sk-1")
    );
}

#[test]
fn a_lost_key_is_never_replaced_while_secrets_are_stored() {
    let f = fixture();
    let a = f.bot("A");
    let b = f.bot("B");
    f.store.set_secret(&a.id, "API_KEY", "sk-1", None).unwrap();

    let key = Arc::new(TestKey::default());
    let restored =
        LocalBotStore::open(&f.path, key.clone(), Arc::new(Recorder::new(&f.path))).unwrap();
    let refused = restored
        .set_secret(&b.id, "OTHER_KEY", "sk-2", None)
        .unwrap_err();
    assert_eq!(refused.code(), codes::SECRET_KEY);
    assert!(
        refused.message().contains("delete the old secrets first"),
        "{refused}"
    );
    assert!(!refused.message().contains("sk-2"), "{refused}");
    assert_eq!(key.calls(), [false], "asked without creating a second key");
    assert!(restored.secrets(&b.id).unwrap().is_empty());

    restored.delete_secret(&a.id, "API_KEY").unwrap();
    restored
        .set_secret(&b.id, "OTHER_KEY", "sk-2", None)
        .unwrap();
    assert_eq!(
        key.calls(),
        [false, true],
        "a key is made once no secret depends on the lost one"
    );
    assert_eq!(
        opened_value(&restored, &b.id, "OTHER_KEY").as_deref(),
        Some("sk-2")
    );
}

#[test]
fn opening_secrets_for_the_upload_needs_the_bot_and_asks_the_keychain_only_for_secrets() {
    let f = fixture();
    let quiet = f.bot("Quiet");
    let opened = f.store.open_secrets(&quiet.id).unwrap();
    assert!(opened.readable.is_empty() && opened.unreadable.is_empty());
    assert!(f.key.calls().is_empty());
    assert_eq!(code(f.store.open_secrets("bot_missing")), codes::NOT_FOUND);
}

#[test]
fn secret_names_values_and_counts_follow_the_api() {
    let f = fixture();
    let bot = f.bot("Keeper");
    for valid in ["A", "API_KEY", "X9_", &format!("A{}", "B".repeat(63))] {
        assert!(
            f.store.set_secret(&bot.id, valid, "v", None).is_ok(),
            "{valid}"
        );
    }
    for invalid in [
        "",
        "api_key",
        "_KEY",
        "9KEY",
        "API-KEY",
        "ÄPI",
        &format!("A{}", "B".repeat(64)),
    ] {
        let error = f.store.set_secret(&bot.id, invalid, "v", None);
        assert_eq!(code(error), codes::INVALID, "{invalid}");
    }

    let calls = f.key.calls().len();
    assert!(
        f.store
            .set_secret(&bot.id, "BIG", &"x".repeat(16 * 1024), None)
            .is_ok()
    );
    let oversized = f
        .store
        .set_secret(&bot.id, "BIGGER", &"x".repeat(16 * 1024 + 1), None)
        .unwrap_err();
    assert!(oversized.message().contains("bytes"), "{oversized}");
    let long_description = Some(Some("d".repeat(2_001)));
    let described = f
        .store
        .set_secret(&bot.id, "DESCRIBED", "v", long_description);
    assert_eq!(code(described), codes::INVALID);
    let missing_bot = f.store.set_secret("bot_missing", "API_KEY", "v", None);
    assert_eq!(code(missing_bot), codes::NOT_FOUND);
    assert_eq!(
        f.key.calls().len(),
        calls + 1,
        "refused writes never reach the keychain"
    );
}

#[test]
fn a_bot_holds_at_most_fifty_secrets() {
    let f = fixture();
    let bot = f.bot("Keeper");
    for n in 0..50 {
        f.store
            .set_secret(&bot.id, &format!("KEY_{n}"), "v", None)
            .unwrap();
    }
    let error = f
        .store
        .set_secret(&bot.id, "KEY_50", "v", None)
        .unwrap_err();
    assert_eq!(error.code(), codes::BOT_SECRET_LIMIT_REACHED);
    f.store
        .set_secret(&bot.id, "KEY_7", "replaced", None)
        .unwrap();
    f.store.delete_secret(&bot.id, "KEY_7").unwrap();
    f.store.set_secret(&bot.id, "KEY_50", "v", None).unwrap();

    let names: Vec<String> = f
        .store
        .secrets(&bot.id)
        .unwrap()
        .into_iter()
        .map(|secret| secret.name)
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "sorted by name");
}

#[test]
fn deleting_a_secret_is_idempotent_for_an_existing_bot() {
    let f = fixture();
    let bot = f.bot("Keeper");
    f.store.set_secret(&bot.id, "API_KEY", "v", None).unwrap();
    f.changes.take();
    f.store.delete_secret(&bot.id, "API_KEY").unwrap();
    assert_eq!(f.changes.take(), [Change::Bots(Some(bot.id.clone()))]);
    f.store.delete_secret(&bot.id, "API_KEY").unwrap();
    assert!(f.changes.take().is_empty());
    assert!(f.store.secrets(&bot.id).unwrap().is_empty());
    assert_eq!(
        code(f.store.delete_secret("bot_missing", "API_KEY")),
        codes::NOT_FOUND
    );
    assert_eq!(code(f.store.secrets("bot_missing")), codes::NOT_FOUND);
}

#[test]
fn a_keychain_that_refuses_stores_nothing() {
    let f = fixture_with(TestKey::refusing());
    let bot = f.bot("Keeper");
    f.changes.take();
    let error = f
        .store
        .set_secret(&bot.id, "API_KEY", "sk-1", None)
        .unwrap_err();
    assert_eq!(error.code(), codes::SECRET_KEY);
    assert!(
        error.message().contains("the keychain is locked"),
        "{error}"
    );
    assert!(f.store.secrets(&bot.id).unwrap().is_empty());
    assert!(f.changes.take().is_empty());
}
