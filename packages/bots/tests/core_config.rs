use flow_like_bots::config::{DEFAULT_DISCORD_INTENTS, DISCORD_INTENTS};
use flow_like_bots::{BotProblem, BotSettings, BotSpec, BotToken, Message, Provider};
use serde_json::{Map, Value, json};

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
    assert_eq!(telegram.command_prefix, "");
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
    assert_eq!(discord.command_prefix, "!");
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
    assert_eq!(telegram.command_prefix, "");

    let discord = spec(
        "discord",
        json!({"intents":null,"chat_whitelist":5,"respond_to_private":"x"}),
    )
    .unwrap();
    assert_eq!(discord.intents.len(), 3);
}

#[test]
fn absent_null_and_empty_prefixes_are_no_prefix() {
    for event_type in ["telegram", "discord"] {
        let absent = spec(event_type, json!({})).unwrap();
        assert_eq!(absent.command_prefix, "", "{event_type}");
        for prefix in [Value::Null, json!("")] {
            let read = spec(event_type, json!({ "command_prefix": prefix })).unwrap();
            assert_eq!(read, absent, "{event_type}: {prefix}");
        }
        let blank = spec(event_type, json!({"command_prefix": " "})).unwrap();
        assert_eq!(blank.command_prefix, " ", "{event_type}: untrimmed");
        assert!(blank.admits(&message("-5", false, false, " ask")));
        assert!(!blank.admits(&message("-5", false, false, "ask")));
    }
}

#[test]
fn a_prefix_is_a_text_of_at_most_sixteen_characters_for_both_providers() {
    for event_type in ["telegram", "discord"] {
        let longest = "🙂".repeat(16);
        let read = spec(event_type, json!({ "command_prefix": longest })).unwrap();
        assert_eq!(read.command_prefix, longest, "{event_type}");
        let wrong = [
            json!("x".repeat(17)),
            json!(5),
            json!(["!"]),
            json!(true),
            json!({"prefix": "!"}),
        ];
        for prefix in wrong {
            let config = json!({ "command_prefix": prefix });
            refused(event_type, config, "command_prefix");
        }
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
    refused("discord", json!({"intents": "Guilds"}), "intents");
    refused("discord", json!({"intents": [1]}), "intents");
}

/// With every key from one on wrong, that one is named: lists, `respond_to_mentions`, the
/// private flag, the prefix, then the intents of Discord.
#[test]
fn the_first_problem_is_the_one_a_client_names() {
    let telegram = [
        "chat_whitelist",
        "chat_blacklist",
        "respond_to_mentions",
        "respond_to_private",
        "command_prefix",
    ];
    let discord = [
        "channel_whitelist",
        "channel_blacklist",
        "respond_to_mentions",
        "respond_to_dms",
        "command_prefix",
        "intents",
    ];
    for (event_type, order) in [("telegram", &telegram[..]), ("discord", &discord[..])] {
        for first in 0..order.len() {
            let wrong: Map<String, Value> = order[first..]
                .iter()
                .map(|key| (key.to_string(), json!(7)))
                .collect();
            refused(event_type, Value::Object(wrong), order[first]);
        }
    }
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
    let slash = spec("telegram", json!({"command_prefix": "/"})).unwrap();
    assert!(slash.admits(&message("1", true, false, "hello")));
    assert!(slash.admits(&message("-5", false, true, "hey bot")));
    assert!(slash.admits(&message("-5", false, false, "/start")));
    assert!(slash.admits(&message("-5", false, false, "/start@thisbot")));
    let mut foreign = message("-5", false, false, "/start@otherbot");
    foreign.foreign_command = true;
    assert!(!slash.admits(&foreign));
    assert!(!slash.admits(&message("-5", false, false, "just talking")));
    let inside = message("-5", false, false, "see /start");
    assert!(!slash.admits(&inside), "the prefix leads the text");

    let quiet = spec(
        "telegram",
        json!({"respond_to_private": false, "respond_to_mentions": false, "command_prefix": ""}),
    )
    .unwrap();
    assert!(!quiet.admits(&message("1", true, false, "hello")));
    assert!(quiet.admits(&message("-5", false, true, "hey bot")));
    assert!(quiet.admits(&message("-5", false, false, "just talking")));
    assert!(quiet.admits(&foreign), "every message, without a prefix");

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

const GROUP: &str = "-5";
const DENIED: &str = "-9";

/// A row of rule R's table: the prefix (`None`: the key is absent), "respond to mentions",
/// and whether a group message starts a run that is plain, starts with the prefix, or is
/// addressed to the bot (a mention, a reply).
struct Row {
    provider: Provider,
    prefix: Option<&'static str>,
    mentions: bool,
    plain: bool,
    prefixed: bool,
    addressed: bool,
}

const fn row(
    provider: Provider,
    prefix: Option<&'static str>,
    mentions: bool,
    [plain, prefixed, addressed]: [bool; 3],
) -> Row {
    Row {
        provider,
        prefix,
        mentions,
        plain,
        prefixed,
        addressed,
    }
}

const RULE: [Row; 8] = [
    row(Provider::Telegram, Some("/"), true, [false, true, true]),
    row(Provider::Discord, Some("!"), true, [false, true, true]),
    row(Provider::Telegram, Some("/"), false, [false, true, false]),
    row(Provider::Discord, Some("!"), false, [false, true, false]),
    row(Provider::Telegram, None, true, [true, true, true]),
    row(Provider::Telegram, None, false, [true, true, true]),
    row(Provider::Discord, None, true, [false, false, true]),
    row(Provider::Discord, None, false, [true, true, true]),
];

impl Row {
    fn name(&self) -> String {
        let Self {
            provider,
            prefix,
            mentions,
            ..
        } = self;
        format!("{provider:?}, prefix {prefix:?}, mentions {mentions}")
    }

    /// The row's bot; chat `DENIED` is on its deny list.
    fn bot(&self, private: bool) -> BotSpec {
        let (deny, private_key) = match self.provider {
            Provider::Telegram => ("chat_blacklist", "respond_to_private"),
            Provider::Discord => ("channel_blacklist", "respond_to_dms"),
        };
        let mut config =
            json!({ deny: [DENIED], "respond_to_mentions": self.mentions, private_key: private });
        if let Some(prefix) = self.prefix {
            config["command_prefix"] = json!(prefix);
        }
        spec(self.provider.as_str(), config).unwrap()
    }

    /// Plain, starting with the prefix, mentioning the bot and replying to it, with what the
    /// row says of each. Without a prefix the second starts with the editor's default one.
    fn group_messages(&self, chat: &str) -> [(&'static str, Message, bool); 4] {
        let editor = match self.provider {
            Provider::Telegram => "/",
            Provider::Discord => "!",
        };
        let command = format!("{}ask now", self.prefix.unwrap_or(editor));
        [
            ("plain", message(chat, false, false, "hello"), self.plain),
            (
                "starts with the prefix",
                message(chat, false, false, &command),
                self.prefixed,
            ),
            (
                "mentions the bot",
                message(chat, false, true, "hey @helper_bot"),
                self.addressed,
            ),
            (
                "replies to the bot",
                message(chat, false, true, "yes"),
                self.addressed,
            ),
        ]
    }
}

#[test]
fn rule_truth_table() {
    for row in &RULE {
        let (name, bot) = (row.name(), row.bot(true));
        for (case, message, starts) in row.group_messages(GROUP) {
            assert_eq!(bot.admits(&message), starts, "{name}: {case}");
        }
        for (case, message, _) in row.group_messages(DENIED) {
            assert!(!bot.admits(&message), "{name}: {case}, in a denied chat");
        }
        let private = message("42", true, false, "hello");
        assert!(bot.admits(&private), "{name}: private");
        assert!(!row.bot(false).admits(&private), "{name}: private, off");
        let denied = message(DENIED, true, false, "hello");
        assert!(!bot.admits(&denied), "{name}: private, denied");
        if row.provider == Provider::Telegram {
            let mut foreign = message(GROUP, false, false, "/ask@otherbot now");
            foreign.foreign_command = true;
            let starts = row.prefix.is_none();
            assert_eq!(bot.admits(&foreign), starts, "{name}: a foreign command");
        }
    }
}

#[test]
fn unbounded_takes_the_desktop_apps_settings_as_they_are() {
    let prefix = "!".repeat(40);
    let settings = BotSettings {
        allow: (0..300).map(|id| id.to_string()).collect(),
        deny: vec!["299".into(), "x".repeat(65), String::new()],
        respond_to_mentions: false,
        respond_to_private: false,
        command_prefix: prefix.clone(),
    };
    let bot = BotSpec::unbounded(&"e".repeat(113), Provider::Discord, settings);
    assert_eq!(bot.event_id.len(), 113);
    assert_eq!((bot.allow.len(), bot.deny.len(), bot.open), (300, 3, false));
    assert_eq!(bot.command_prefix, prefix);
    assert!(bot.intents.is_empty());

    let command = format!("{prefix}go");
    assert!(bot.admits(&message("298", false, false, &command)));
    assert!(!bot.admits(&message("298", false, true, "hey")));
    assert!(!bot.admits(&message("299", false, false, &command)));
    assert!(!bot.admits(&message("300", false, false, &command)));
    assert!(!bot.admits(&message("298", true, false, "hey")));
}

#[test]
fn unbounded_and_a_devices_reading_build_the_same_bot() {
    let telegram = json!({"chat_whitelist": ["1", "2"], "chat_blacklist": ["3"],
        "respond_to_mentions": false, "respond_to_private": false, "command_prefix": "!ask"});
    let discord = json!({"channel_whitelist": ["1", "2"], "channel_blacklist": ["3"],
        "respond_to_mentions": false, "respond_to_dms": false, "command_prefix": "!ask",
        "intents": []});
    for (provider, config) in [(Provider::Telegram, telegram), (Provider::Discord, discord)] {
        let settings = BotSettings {
            allow: vec!["1".into(), "2".into()],
            deny: vec!["3".into()],
            respond_to_mentions: false,
            respond_to_private: false,
            command_prefix: "!ask".into(),
        };
        let built = BotSpec::unbounded("evt_helper", provider, settings);
        assert_eq!(built, spec(provider.as_str(), config).unwrap());
    }
    let open = BotSettings {
        allow: Vec::new(),
        deny: Vec::new(),
        respond_to_mentions: true,
        respond_to_private: true,
        command_prefix: String::new(),
    };
    let built = BotSpec::unbounded("evt_helper", Provider::Telegram, open);
    assert_eq!(built, spec("telegram", json!({})).unwrap());
}
