use flow_like_bots::config::{DEFAULT_DISCORD_INTENTS, DISCORD_INTENTS};
use flow_like_bots::{BotProblem, BotSpec, BotToken, Message, Provider};
use serde_json::{Value, json};

const TELEGRAM_TOKEN: &str = "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw";
// Synthetic segments exercise local parsing without resembling an issued token.
const DISCORD_TOKEN: &str = "MTIzNDU2Nzg5MDEyMzQ1Njc4.test-only.not-a-real-discord-token";

fn spec(event_type: &str, config: Value) -> Result<BotSpec, BotProblem> {
    BotSpec::from_config("evt_helper", event_type, config.to_string().as_bytes())
}

fn refused(event_type: &str, config: Value, key: &str) {
    let problem = spec(event_type, config.clone()).expect_err(&config.to_string());
    assert_eq!(problem.code(), "bot_invalid", "{config}");
    assert!(
        problem.to_string().contains(key),
        "{config}: {problem} does not name {key}"
    );
}

#[test]
fn defaults_per_provider() {
    let telegram = spec("telegram", json!({})).unwrap();
    assert_eq!(telegram.provider, Provider::Telegram);
    assert!(telegram.open);
    assert!(telegram.allow.is_empty() && telegram.deny.is_empty());
    assert!(telegram.respond_to_mentions && telegram.respond_to_private);
    assert_eq!(telegram.command_prefix, "/");
    assert!(telegram.intents.is_empty());

    let discord = spec("discord", json!({})).unwrap();
    assert_eq!(discord.provider, Provider::Discord);
    assert_eq!(discord.command_prefix, "");
    assert_eq!(discord.intents, DEFAULT_DISCORD_INTENTS.map(str::to_string));
    assert!(discord.open && discord.respond_to_mentions && discord.respond_to_private);
}

#[test]
fn saved_editor_config_reads() {
    let telegram = spec(
        "telegram",
        json!({"sink_type":"telegram","bot_token":"","bot_name":"Flow-Like Bot","bot_description":"",
               "chat_whitelist":["-1001234567890"],"chat_blacklist":["42"],"respond_to_mentions":false,
               "respond_to_private":false,"command_prefix":"!ask","token":"x","webhook_secret":"y"}),
    )
    .unwrap();
    assert!(!telegram.open);
    assert_eq!(telegram.allow, ["-1001234567890"]);
    assert_eq!(telegram.deny, ["42"]);
    assert!(!telegram.respond_to_mentions && !telegram.respond_to_private);
    assert_eq!(telegram.command_prefix, "!ask");

    let discord = spec(
        "discord",
        json!({"sink_type":"discord","token":"","intents":["Guilds","GuildMessages","MessageContent","DirectMessages","Guilds"],
               "channel_whitelist":[],"channel_blacklist":["9"],"respond_to_mentions":true,"respond_to_dms":false,
               "command_prefix":"!"}),
    )
    .unwrap();
    assert!(discord.open);
    assert_eq!(
        discord.intents,
        [
            "Guilds",
            "GuildMessages",
            "MessageContent",
            "DirectMessages"
        ]
    );
    assert!(!discord.respond_to_private);
    assert_eq!(discord.command_prefix, "");
}

#[test]
fn null_is_absent_and_other_providers_keys_are_ignored() {
    let telegram = spec(
        "telegram",
        json!({"chat_whitelist":null,"respond_to_mentions":null,"command_prefix":null,
               "channel_whitelist":5,"intents":"bad","respond_to_dms":"x"}),
    )
    .unwrap();
    assert!(telegram.open && telegram.respond_to_mentions);
    assert_eq!(telegram.command_prefix, "/");

    let discord = spec(
        "discord",
        json!({"intents":null,"chat_whitelist":5,"respond_to_private":"x"}),
    )
    .unwrap();
    assert_eq!(discord.intents.len(), 3);
}

#[test]
fn discord_reads_no_prefix() {
    for prefix in [json!(["!"]), json!(5), json!("x".repeat(17))] {
        let discord = spec("discord", json!({ "command_prefix": prefix })).unwrap();
        assert_eq!(discord.command_prefix, "");
    }
}

#[test]
fn wrong_types_name_their_key() {
    for (event_type, key) in [
        ("telegram", "chat_whitelist"),
        ("telegram", "chat_blacklist"),
        ("discord", "channel_whitelist"),
        ("discord", "channel_blacklist"),
    ] {
        refused(event_type, json!({ key: "123" }), key);
        refused(event_type, json!({ key: [123] }), key);
        refused(event_type, json!({ key: {"a": 1} }), key);
    }
    for (event_type, key) in [
        ("telegram", "respond_to_mentions"),
        ("telegram", "respond_to_private"),
        ("discord", "respond_to_mentions"),
        ("discord", "respond_to_dms"),
    ] {
        refused(event_type, json!({ key: "yes" }), key);
        refused(event_type, json!({ key: 1 }), key);
    }
    refused("telegram", json!({"command_prefix": 5}), "command_prefix");
    refused(
        "telegram",
        json!({"command_prefix": ["/"]}),
        "command_prefix",
    );
    refused("discord", json!({"intents": "Guilds"}), "intents");
    refused("discord", json!({"intents": [1]}), "intents");
}

#[test]
fn the_first_problem_is_the_one_a_client_names() {
    refused(
        "discord",
        json!({"respond_to_dms": 1, "intents": "Guilds"}),
        "respond_to_dms",
    );
    refused(
        "telegram",
        json!({"chat_blacklist": 1, "command_prefix": 5}),
        "chat_blacklist",
    );
}

#[test]
fn bounds() {
    let ids = |count: usize| (0..count).map(|id| id.to_string()).collect::<Vec<_>>();
    assert_eq!(
        spec("telegram", json!({"chat_whitelist": ids(256)}))
            .unwrap()
            .allow
            .len(),
        256
    );
    refused(
        "telegram",
        json!({"chat_whitelist": ids(257)}),
        "chat_whitelist",
    );
    refused(
        "discord",
        json!({"channel_blacklist": [""]}),
        "channel_blacklist",
    );
    let wide = "ä".repeat(64);
    assert_eq!(
        spec("discord", json!({"channel_whitelist": [wide]}))
            .unwrap()
            .allow
            .len(),
        1
    );
    refused(
        "discord",
        json!({"channel_whitelist": ["ä".repeat(65)]}),
        "channel_whitelist",
    );
    assert_eq!(
        spec("telegram", json!({"command_prefix": "🙂".repeat(16)}))
            .unwrap()
            .command_prefix
            .chars()
            .count(),
        16
    );
    refused(
        "telegram",
        json!({"command_prefix": "x".repeat(17)}),
        "command_prefix",
    );
}

#[test]
fn intents() {
    let all = spec("discord", json!({ "intents": DISCORD_INTENTS })).unwrap();
    assert_eq!(all.intents.len(), 19);
    assert!(
        spec("discord", json!({"intents": []}))
            .unwrap()
            .intents
            .is_empty()
    );
    let problem = spec("discord", json!({"intents": ["Guilds", "Everything"]})).unwrap_err();
    assert_eq!(problem.code(), "bot_invalid");
    assert!(problem.to_string().contains("Everything"), "{problem}");
}

#[test]
fn not_a_bot_or_not_an_object() {
    for config in [&b""[..], b"[]", b"\"x\"", b"null", b"{", b"7"] {
        let problem = BotSpec::from_config("evt", "telegram", config).unwrap_err();
        assert_eq!(problem.code(), "bot_invalid");
    }
    assert_eq!(spec("cron", json!({})).unwrap_err().code(), "bot_invalid");
    assert!(BotSpec::from_config(&"e".repeat(112), "discord", b"{}").is_ok());
    let problem = BotSpec::from_config(&"e".repeat(113), "discord", b"{}").unwrap_err();
    assert_eq!(problem.code(), "bot_invalid");
}

#[test]
fn token_shapes() {
    let telegram = BotToken::parse(Provider::Telegram, TELEGRAM_TOKEN.as_bytes()).unwrap();
    assert_eq!(telegram.expose(), TELEGRAM_TOKEN);
    assert_eq!(telegram.bot_id(), Some(123_456_789));
    let quoted = serde_json::to_vec(&format!(" {TELEGRAM_TOKEN}\n")).unwrap();
    assert_eq!(
        BotToken::parse(Provider::Telegram, &quoted)
            .unwrap()
            .expose(),
        TELEGRAM_TOKEN
    );
    assert_eq!(
        BotToken::parse(Provider::Telegram, format!("{TELEGRAM_TOKEN}\n").as_bytes())
            .unwrap()
            .expose(),
        TELEGRAM_TOKEN
    );
    for (secret, code) in [
        ("", "bot_token_missing"),
        ("  \n", "bot_token_missing"),
        ("\"\"", "bot_token_missing"),
        ("please paste the token here", "bot_token_invalid"),
        ("12:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw", "bot_token_invalid"),
        ("123456789:AAHdqTcvCH1vGWJxfSe", "bot_token_invalid"),
        (
            "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALD!aw",
            "bot_token_invalid",
        ),
        (DISCORD_TOKEN, "bot_token_invalid"),
    ] {
        let problem = BotToken::parse(Provider::Telegram, secret.as_bytes()).unwrap_err();
        assert_eq!(problem.code(), code, "{secret:?}");
        let sentence = problem.to_string();
        assert!(
            !sentence.contains("AAHdq") && !sentence.contains("paste"),
            "{sentence}"
        );
    }

    let discord = BotToken::parse(Provider::Discord, DISCORD_TOKEN.as_bytes()).unwrap();
    assert_eq!(discord.bot_id(), Some(123_456_789_012_345_678));
    for secret in [
        "MTIzNDU2Nzg5MDEyMzQ1Njc4.GabcDE",
        "MTIz.Ga.abc",
        "MTIzNDU2Nzg5MDEyMzQ1Njc4.GabcDE.abcdefghij+lmnopqrstuvwxyz0123456789ABCD",
        "MTIzNDU2Nzg5MDEyMzQ1Njc4..abcdefghijklmnopqrstuvwxyz0123456789ABCD",
        TELEGRAM_TOKEN,
    ] {
        assert_eq!(
            BotToken::parse(Provider::Discord, secret.as_bytes())
                .unwrap_err()
                .code(),
            "bot_token_invalid",
            "{secret}"
        );
    }
    let long = format!("{}.{}.{}", "a".repeat(100), "b".repeat(100), "c".repeat(57));
    assert_eq!(
        BotToken::parse(Provider::Discord, long.as_bytes())
            .unwrap_err()
            .code(),
        "bot_token_invalid"
    );
}

#[test]
fn tokens_never_print() {
    let token = BotToken::parse(Provider::Telegram, TELEGRAM_TOKEN.as_bytes()).unwrap();
    let printed = format!("{token:?} {:?}", Some(&token));
    assert!(
        !printed.contains("AAHdq") && !printed.contains("123456789"),
        "{printed}"
    );
}

#[test]
fn settings_print_no_chat_ids() {
    let settings = spec(
        "telegram",
        json!({"chat_whitelist": ["-1009876"], "chat_blacklist": ["5551234"]}),
    )
    .unwrap();
    let printed = format!("{settings:?}");
    assert!(
        !printed.contains("9876") && !printed.contains("5551234"),
        "{printed}"
    );
    let mut message = Message::new("-1009876", ());
    message.text = "secret words".into();
    let printed = format!("{message:?}");
    assert!(
        !printed.contains("9876") && !printed.contains("secret"),
        "{printed}"
    );
}

fn message(chat: &str, private: bool, addressed: bool, text: &str) -> Message {
    let mut message = Message::new(chat, ());
    message.private = private;
    message.addressed = addressed;
    message.text = text.to_string();
    message
}

#[test]
fn filter_rules_three_to_five() {
    let open = spec("telegram", json!({})).unwrap();
    assert!(open.admits(&message("1", true, false, "hello")));
    assert!(open.admits(&message("-5", false, true, "hey bot")));
    assert!(open.admits(&message("-5", false, false, "/start")));
    assert!(open.admits(&message("-5", false, false, "/start@thisbot")));
    let mut foreign = message("-5", false, false, "/start@otherbot");
    foreign.foreign_command = true;
    assert!(!open.admits(&foreign));
    assert!(!open.admits(&message("-5", false, false, "just talking")));

    let quiet = spec(
        "telegram",
        json!({"respond_to_private": false, "respond_to_mentions": false, "command_prefix": ""}),
    )
    .unwrap();
    assert!(!quiet.admits(&message("1", true, false, "hello")));
    assert!(!quiet.admits(&message("-5", false, true, "hey bot")));
    assert!(!quiet.admits(&message("-5", false, false, "/start")));

    let listed = spec(
        "telegram",
        json!({"chat_whitelist": ["1", "-5"], "chat_blacklist": ["-5"]}),
    )
    .unwrap();
    assert!(listed.admits(&message("1", true, false, "hi")));
    assert!(!listed.admits(&message("2", true, false, "hi")));
    assert!(!listed.admits(&message("-5", false, true, "hi")));
    let denied = spec("telegram", json!({"chat_blacklist": ["3"]})).unwrap();
    assert!(!denied.admits(&message("3", true, false, "hi")));
    assert!(denied.admits(&message("4", true, false, "hi")));
}

#[test]
fn discord_follows_the_desktop_rule_in_servers() {
    let mention_only = spec("discord", json!({"command_prefix": "!"})).unwrap();
    assert!(mention_only.admits(&message("7", false, true, "hey")));
    assert!(!mention_only.admits(&message("7", false, false, "hey")));
    assert!(!mention_only.admits(&message("7", false, false, "!ask hey")));

    let every = spec(
        "discord",
        json!({"respond_to_mentions": false, "respond_to_dms": false, "channel_blacklist": ["9"]}),
    )
    .unwrap();
    assert!(every.admits(&message("7", false, false, "hey")));
    assert!(every.admits(&message("7", false, true, "hey")));
    assert!(!every.admits(&message("9", false, false, "hey")));
    assert!(!every.admits(&message("7", true, false, "hey")));
}
