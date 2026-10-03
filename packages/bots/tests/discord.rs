//! The Discord provider's decisions, from literal gateway JSON, without a gateway (design §7.8).
#![cfg(feature = "discord")]

use flow_like_bots::config::DISCORD_INTENTS;
use flow_like_bots::discord::{
    ClientEnd, Verdict, answer_text, chat_payload, facts, facts_of, final_parts, intent, intents,
    progress_text, stage_link, verdict,
};
use flow_like_bots::render::utf16_len;
use flow_like_bots::reply::{Answer, Link};
use flow_like_bots::runner::{End, LinkState, Refusal};
use flow_like_bots::{BotSpec, Provider};
use serde_json::{Value, json};
use serenity::gateway::{ConnectionStage, GatewayError};
use serenity::model::channel::Message;
use serenity::model::event::Event;
use serenity::model::gateway::GatewayIntents;
use serenity::model::id::UserId;

const BOT: u64 = 900;
const HANDLE: &str = "device-bot:evt_dc";
/// Discord's limit for one message.
const MESSAGE_LIMIT: usize = 2_000;

fn bot() -> Option<UserId> {
    Some(UserId::new(BOT))
}

/// A `MESSAGE_CREATE` message as the gateway sends it, with `extra` keys replaced.
fn message_json(id: u64, extra: Value) -> Value {
    let mut message = json!({
        "id": id.to_string(),
        "channel_id": "300",
        "guild_id": "400",
        "author": {"id": "500", "username": "alice", "global_name": "Alice", "discriminator": "0"},
        "member": {"nick": "Ali", "roles": []},
        "content": "hello",
        "timestamp": "2026-10-02T10:00:00.000000+00:00",
        "edited_timestamp": null,
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0
    });
    for (key, value) in extra.as_object().expect("extra keys are an object") {
        message[key] = value.clone();
    }
    message
}

fn message(id: u64, extra: Value) -> Message {
    serde_json::from_value(message_json(id, extra)).expect("a gateway message")
}

fn gateway(kind: &str, data: Value) -> Event {
    serde_json::from_value(json!({"t": kind, "d": data})).expect("a gateway event")
}

fn created(extra: Value) -> Event {
    gateway("MESSAGE_CREATE", message_json(1000, extra))
}

fn attachment(id: u64, name: &str, content_type: &str) -> Value {
    json!({
        "id": id.to_string(), "filename": name, "size": 2048, "content_type": content_type,
        "url": format!("https://cdn.discordapp.com/attachments/300/{id}/{name}"),
        "proxy_url": format!("https://media.discordapp.net/attachments/300/{id}/{name}")
    })
}

fn bot_user() -> Value {
    json!({"id": BOT.to_string(), "username": "helper", "bot": true, "discriminator": "0"})
}

fn sticker() -> Value {
    json!([{"id": "5", "name": "wave", "format_type": 1}])
}

#[derive(Debug, PartialEq)]
enum Seen {
    Candidate,
    Ignored,
    Unrelated,
}

fn expect(cases: Vec<(&str, Event)>, expected: Seen) {
    for (case, event) in cases {
        let seen = match verdict(&event, bot()) {
            Verdict::Candidate(_) => Seen::Candidate,
            Verdict::Ignored => Seen::Ignored,
            Verdict::Unrelated => Seen::Unrelated,
        };
        assert_eq!(seen, expected, "{case}");
    }
}

#[test]
fn filter_starts_new_messages_of_people() {
    let replied = message_json(990, json!({"author": bot_user(), "content": "an answer"}));
    let image = attachment(1, "cat.png", "image/png");
    expect(
        vec![
            ("text", created(json!({}))),
            (
                "a reply",
                created(json!({"type": 19, "referenced_message": replied})),
            ),
            (
                "an image alone",
                created(json!({"content": "", "attachments": [image]})),
            ),
        ],
        Seen::Candidate,
    );
}

#[test]
fn filter_ignores_system_messages() {
    expect(
        vec![
            (
                "a member joined",
                created(json!({"type": 7, "content": ""})),
            ),
            (
                "a message was pinned",
                created(json!({"type": 6, "content": ""})),
            ),
            ("a slash command's answer", created(json!({"type": 20}))),
            (
                "a thread was created",
                created(json!({"type": 18, "content": "topic"})),
            ),
        ],
        Seen::Ignored,
    );
}

#[test]
fn filter_ignores_bots_webhooks_and_messages_without_text_or_image() {
    let other_bot = json!({"id": "501", "username": "otherbot", "bot": true});
    let hook = json!({"id": "777", "username": "Deploy hook", "discriminator": "0000"});
    let pdf = attachment(2, "report.pdf", "application/pdf");
    expect(
        vec![
            ("another bot", created(json!({"author": other_bot}))),
            (
                "a webhook",
                created(json!({"webhook_id": "777", "author": hook})),
            ),
            (
                "a sticker alone",
                created(json!({"content": "", "sticker_items": sticker()})),
            ),
            (
                "a file alone",
                created(json!({"content": "", "attachments": [pdf]})),
            ),
            ("blank text", created(json!({"content": "  \n "}))),
        ],
        Seen::Ignored,
    );
}

#[test]
fn filter_leaves_edits_and_the_bots_own_messages_out() {
    let edited_at = "2026-10-02T10:01:00.000000+00:00";
    let edit = message_json(
        1000,
        json!({"content": "again", "edited_timestamp": edited_at}),
    );
    let preview = json!({"id": "1000", "channel_id": "300", "guild_id": "400", "embeds": []});
    let typing = json!({"channel_id": "300", "user_id": "500", "timestamp": 1790000000});
    let own = json!({"author": bot_user(), "content": "Here you go"});
    expect(
        vec![
            ("an edit", gateway("MESSAGE_UPDATE", edit)),
            ("a link preview", gateway("MESSAGE_UPDATE", preview)),
            ("the bot's own answer", created(own)),
            ("someone is typing", gateway("TYPING_START", typing)),
        ],
        Seen::Unrelated,
    );
}

fn spec(config: Value) -> BotSpec {
    BotSpec::from_config("evt_dc", "discord", config.to_string().as_bytes())
        .expect("valid settings")
}

fn admitted(spec: &BotSpec, extra: Value) -> bool {
    spec.admits(&facts(message(1000, extra), bot()))
}

/// A reply to message 990 of `author`.
fn reply_to(author: Value) -> Value {
    let replied = message_json(990, json!({"author": author, "content": "an answer"}));
    json!({"type": 19, "referenced_message": replied})
}

fn bob() -> Value {
    json!({"id": "502", "username": "bob"})
}

#[test]
fn filter_answers_mentions_and_replies_in_servers_without_a_prefix() {
    let defaults = spec(json!({}));

    assert!(!admitted(&defaults, json!({})), "not addressed");
    assert!(admitted(&defaults, json!({"mentions": [bot_user()]})));
    assert!(!admitted(&defaults, json!({"mentions": [bob()]})));
    assert!(admitted(&defaults, reply_to(bot_user())));
    assert!(!admitted(&defaults, reply_to(bob())));
    assert!(
        !admitted(&defaults, json!({"content": "!ask now"})),
        "the editor's default prefix is not set here"
    );
    for none in [json!(null), json!("")] {
        let unset = spec(json!({ "command_prefix": none }));
        assert!(!admitted(&unset, json!({"content": "!ask now"})));
        assert!(admitted(&unset, json!({"mentions": [bot_user()]})));
    }
}

#[test]
fn filter_answers_the_prefix_mentions_and_replies_in_servers() {
    let prefixed = spec(json!({"command_prefix": "?"}));

    assert!(admitted(&prefixed, json!({"content": "?ask now"})));
    assert!(!admitted(&prefixed, json!({"content": "ask? now"})));
    assert!(!admitted(&prefixed, json!({})), "not addressed");
    assert!(admitted(&prefixed, json!({"mentions": [bot_user()]})));
    assert!(admitted(&prefixed, reply_to(bot_user())));
    assert!(!admitted(&prefixed, reply_to(bob())));

    let image = attachment(1, "cat.png", "image/png");
    let no_text = json!({"content": "", "attachments": [image]});
    assert!(!admitted(&prefixed, no_text), "an image alone");
}

#[test]
fn filter_answers_only_the_prefix_in_servers_with_mentions_off() {
    let only = spec(json!({"command_prefix": "!", "respond_to_mentions": false}));

    assert!(admitted(&only, json!({"content": "!ask now"})));
    assert!(!admitted(&only, json!({})), "not addressed");
    assert!(!admitted(&only, json!({"mentions": [bot_user()]})));
    assert!(!admitted(&only, reply_to(bot_user())));
    let both = json!({"content": "!ask <@900>", "mentions": [bot_user()]});
    assert!(admitted(&only, both));
}

#[test]
fn filter_answers_every_message_in_servers_with_mentions_off_and_no_prefix() {
    let every = spec(json!({"respond_to_mentions": false}));

    assert!(admitted(&every, json!({})), "not addressed");
    assert!(admitted(&every, json!({"mentions": [bob()]})));
    assert!(admitted(&every, json!({"mentions": [bot_user()]})));
    assert!(admitted(&every, json!({"content": "!ask now"})));

    let listed = spec(json!({"respond_to_mentions": false, "channel_whitelist": ["301"]}));
    assert!(!admitted(&listed, json!({})), "another channel");
    assert!(admitted(&listed, json!({"channel_id": "301"})));

    let direct = json!({"guild_id": null, "member": null});
    let no_dms = spec(json!({"respond_to_mentions": false, "respond_to_dms": false}));
    assert!(!admitted(&no_dms, direct));
}

#[test]
fn filter_reads_the_channel_lists_and_direct_messages() {
    let mention = json!([bot_user()]);
    let direct = json!({"guild_id": null, "member": null});

    assert!(
        admitted(&spec(json!({})), direct.clone()),
        "a direct message"
    );
    assert!(!admitted(
        &spec(json!({"respond_to_dms": false})),
        direct.clone()
    ));

    let allow = spec(json!({"channel_whitelist": ["301"]}));
    assert!(!admitted(&allow, json!({"mentions": mention})));
    assert!(admitted(
        &allow,
        json!({"channel_id": "301", "mentions": mention})
    ));

    let deny = spec(json!({"channel_blacklist": ["300"]}));
    assert!(!admitted(&deny, json!({"mentions": mention})));
    assert!(!admitted(&deny, direct));
}

#[test]
fn facts_name_the_channel_and_keep_the_message() {
    let direct = message(
        1000,
        json!({"guild_id": null, "member": null, "content": "hi"}),
    );
    let facts = facts(direct, bot());
    assert_eq!(facts.chat, "300");
    assert!(facts.private);
    assert!(!facts.addressed);
    assert_eq!(facts.text, "hi");
    assert_eq!((facts.update_id, facts.sent_at), (None, None));
    assert!(!facts.foreign_command);
    let native = facts.native::<Message>().map(|native| native.id.get());
    assert_eq!(native, Some(1000));
}

/// What the desktop app calls: the same facts from a message it keeps.
#[test]
fn facts_are_read_by_reference_without_keeping_the_message() {
    let content = "?ask <@900>";
    let mention = message(1000, json!({"content": content, "mentions": [bot_user()]}));
    let read = facts_of(&mention, bot());
    assert_eq!((read.chat.as_str(), read.text.as_str()), ("300", content));
    assert!(read.addressed && !read.private && !read.foreign_command);
    assert!(read.native::<Message>().is_none());
    assert!(read.native::<()>().is_some());
    assert!(!facts_of(&mention, None).addressed, "the bot is not known");

    let kept = facts(mention.clone(), bot());
    assert_eq!(
        (&kept.chat, kept.private, kept.addressed, &kept.text),
        (&read.chat, read.private, read.addressed, &read.text)
    );
    for config in [json!({}), json!({"command_prefix": "?"})] {
        let rule = spec(config);
        assert_eq!(rule.admits(&read), rule.admits(&kept));
    }

    let direct = message(1001, json!({"guild_id": null, "member": null}));
    assert!(facts_of(&direct, bot()).private);
}

fn current() -> Message {
    let files = [
        attachment(70, "chart.png", "image/png"),
        attachment(71, "notes.pdf", "application/pdf"),
    ];
    let content = "What is on <@900>?";
    message(
        1000,
        json!({"content": content, "mentions": [bot_user()], "attachments": files}),
    )
}

/// Twelve messages before the current one and one after it, newest first as Discord answers.
fn history() -> Vec<Message> {
    let extra = |id: u64| match id {
        5 => json!({"author": bot_user(), "member": null, "content": "An answer"}),
        7 => json!({"member": null, "content": "see this",
            "attachments": [attachment(7, "photo.png", "image/png")]}),
        8 => json!({"content": "", "sticker_items": sticker()}),
        9 => json!({"member": null, "author": {"id": "503", "username": "carol"},
            "content": "plain"}),
        _ => json!({"content": format!("message {id}")}),
    };
    let mut history: Vec<Message> = (1..=12).map(|id| message(id, extra(id))).collect();
    history.push(message(1001, json!({"content": "written after"})));
    history.reverse();
    history
}

#[test]
fn payload_carries_the_handle_and_the_session() {
    let payload = chat_payload(&current(), &history(), HANDLE, bot());
    assert_eq!(
        payload["local_session"],
        json!({
            "bot_token": HANDLE,
            "bot_user_id": "900",
            "guild_id": "400",
            "message_id": "1000",
            "channel_id": "300",
            "user": {"id": "500", "name": "alice", "discriminator": null, "bot": false}
        })
    );
    let file = "https://cdn.discordapp.com/attachments/300/71/notes.pdf";
    assert_eq!(payload["attachments"], json!([file]));
}

#[test]
fn payload_history_is_bounded_ordered_and_named_from_the_messages() {
    let payload = chat_payload(&current(), &history(), HANDLE, bot());
    let texts: Vec<&str> = payload["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter_map(|entry| entry["content"][0]["text"].as_str())
        .collect();
    let expected = [
        "Ali[id: 500]: message 3",
        "Ali[id: 500]: message 4",
        "helper[id: 900]: An answer",
        "Ali[id: 500]: message 6",
        "Alice[id: 500]: see this",
        "carol[id: 503]: plain",
        "Ali[id: 500]: message 10",
        "Ali[id: 500]: message 11",
        "Ali[id: 500]: message 12",
        "Ali[id: 500]: What is on <@900>?",
    ];
    assert_eq!(
        texts, expected,
        "ten earlier messages at most, oldest first"
    );
}

#[test]
fn payload_roles_names_and_images() {
    let payload = chat_payload(&current(), &history(), HANDLE, bot());
    let messages = payload["messages"].as_array().expect("messages");
    assert_eq!(messages[2]["role"], "assistant");
    assert!(messages[2].get("name").is_none(), "the bot's own message");
    assert_eq!(
        (&messages[5]["role"], &messages[5]["name"]),
        (&json!("user"), &json!("carol"))
    );
    let photo = "https://cdn.discordapp.com/attachments/300/7/photo.png";
    assert_eq!(
        messages[4]["content"][1],
        json!({"type": "image_url", "image_url": {"url": photo}})
    );
    let last = messages.last().expect("the message itself");
    assert_eq!(
        (&last["role"], &last["name"]),
        (&json!("user"), &json!("alice"))
    );
    let chart = "https://cdn.discordapp.com/attachments/300/70/chart.png";
    assert_eq!(last["content"][1]["image_url"]["url"], chart);
}

#[test]
fn payload_without_history_or_names() {
    let author = json!({"id": "504", "username": "dave", "discriminator": "1234"});
    let direct = message(
        1000,
        json!({"guild_id": null, "member": null, "author": author}),
    );
    let payload = chat_payload(&direct, &[], HANDLE, None);
    assert_eq!(payload["local_session"]["guild_id"], Value::Null);
    assert_eq!(payload["local_session"]["bot_user_id"], Value::Null);
    assert_eq!(payload["local_session"]["user"]["discriminator"], 1234);
    let text = json!({"type": "text", "text": "dave[id: 504]: hello"});
    assert_eq!(
        payload["messages"],
        json!([{"role": "user", "content": [text], "name": "dave"}])
    );
    assert_eq!(payload["attachments"], json!([]));
}

fn answer(text: &str) -> Answer {
    Answer {
        text: text.to_string(),
        plan: None,
        links: Vec::new(),
    }
}

#[test]
fn final_answer_is_split_on_character_boundaries() {
    assert_eq!(Provider::Discord.message_limit(), MESSAGE_LIMIT);
    let text = "Grüße aus Köln 😀 中文字符 ".repeat(260);
    assert!(utf16_len(&text) > 2 * MESSAGE_LIMIT);
    let parts = final_parts(&answer(&text));
    assert!(parts.len() >= 3);
    for part in &parts {
        assert!(
            utf16_len(part) <= MESSAGE_LIMIT,
            "{} units",
            utf16_len(part)
        );
        assert!(part.chars().count() <= MESSAGE_LIMIT);
    }
    assert_eq!(parts.concat(), text.trim_end());

    let emoji = "😀".repeat(MESSAGE_LIMIT);
    let parts = final_parts(&answer(&emoji));
    assert_eq!(parts.len(), 2, "2,000 emoji are 4,000 UTF-16 units");
    assert!(parts.iter().all(|part| utf16_len(part) <= MESSAGE_LIMIT));
    assert_eq!(parts.concat(), emoji);
}

#[test]
fn final_answer_ends_with_its_links() {
    let answer = Answer {
        text: "Your report is ready.".into(),
        plan: None,
        links: vec![Link {
            url: "https://files.example/report.pdf".into(),
            name: Some("Report".into()),
        }],
    };
    assert_eq!(
        answer_text(&answer),
        "Your report is ready.\n\n📎 Attachments:\n- [Report](https://files.example/report.pdf)"
    );
    assert_eq!(final_parts(&answer), vec![answer_text(&answer)]);
    assert!(final_parts(&self::answer("   ")).is_empty());
}

#[test]
fn progress_shows_one_message_of_the_answer() {
    let short = answer("Ein kurzer Zwischenstand …");
    assert_eq!(progress_text(&short), "Ein kurzer Zwischenstand …");

    let full = "😀".repeat(MESSAGE_LIMIT / 2);
    assert_eq!(progress_text(&answer(&full)), full, "exactly one message");

    let long = answer(&"Zwischenstand 😀 ".repeat(400));
    let shown = progress_text(&long);
    assert!(utf16_len(&shown) <= MESSAGE_LIMIT);
    assert!(shown.ends_with('…'));
    let start = shown.trim_end_matches('…').trim_end();
    assert!(long.text.starts_with(start));
}

#[test]
fn stages_map_to_states_after_ready() {
    use ConnectionStage::{Connected, Connecting, Disconnected, Handshake, Identifying, Resuming};
    for stage in [Connected, Connecting, Handshake, Identifying] {
        assert_eq!(stage_link(stage, false), None, "{stage:?} before READY");
    }
    assert_eq!(stage_link(Connected, true), Some(LinkState::Connected));
    for stage in [Connecting, Disconnected, Handshake, Identifying, Resuming] {
        let state = stage_link(stage, true);
        assert_eq!(state, Some(LinkState::Reconnecting), "{stage:?}");
    }
}

fn gateway_end(error: GatewayError) -> ClientEnd {
    ClientEnd::of(&Err(serenity::Error::Gateway(error)))
}

#[test]
fn client_ends_map_to_runner_ends() {
    use GatewayError::*;
    assert_eq!(gateway_end(InvalidAuthentication), ClientEnd::TokenRefused);
    assert_eq!(
        gateway_end(DisallowedGatewayIntents),
        ClientEnd::IntentsRefused
    );
    assert_eq!(
        gateway_end(InvalidGatewayIntents),
        ClientEnd::IntentsRefused
    );
    let others = [
        OverloadedShard,
        Closed(None),
        ReconnectFailure,
        InvalidShardData,
    ];
    for error in others {
        assert_eq!(gateway_end(error), ClientEnd::Ended);
    }
    let other = serenity::Error::Other("the shard queue closed");
    assert_eq!(ClientEnd::of(&Err(other)), ClientEnd::Ended);
    assert_eq!(ClientEnd::of(&Ok(())), ClientEnd::Ended);

    assert_eq!(ClientEnd::TokenRefused.end(), End::Refused(Refusal::Token));
    assert_eq!(
        ClientEnd::IntentsRefused.end(),
        End::Refused(Refusal::Intents)
    );
    assert_eq!(ClientEnd::Ended.end(), End::Ended);
    let codes = [
        ClientEnd::TokenRefused,
        ClientEnd::IntentsRefused,
        ClientEnd::Ended,
    ];
    let codes = codes.map(ClientEnd::code);
    assert_eq!(codes, ["token_refused", "intents_refused", "client_ended"]);
}

#[test]
fn intents_follow_the_settings() {
    let base =
        GatewayIntents::GUILDS | GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;
    let defaults = spec(json!({}));
    assert_eq!(intents([&defaults]), base | GatewayIntents::DIRECT_MESSAGES);

    let no_dms = spec(json!({"respond_to_dms": false}));
    assert_eq!(intents([&no_dms]), base);

    let names = ["GuildMessages", "GuildMessageReactions"];
    let reactions = spec(json!({"respond_to_dms": false, "intents": names}));
    let union = intents([&no_dms, &reactions]);
    assert_eq!(
        union,
        base | GatewayIntents::GUILD_MESSAGE_REACTIONS,
        "every event's"
    );

    let mut all = GatewayIntents::empty();
    for name in DISCORD_INTENTS {
        let flag = intent(name);
        assert!(!flag.is_empty(), "{name}");
        assert!(!all.intersects(flag), "{name} twice");
        all |= flag;
    }
    assert!(intent("Everything").is_empty());
}
