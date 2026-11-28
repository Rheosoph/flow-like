use super::{
    CHANNEL_MESSAGE_PREFIX, ChannelProvider, OutsideAttachment, OutsideMessage, OutsideSender,
    link_id, outside_requester, routes::DiscordMessageRequest, trigger_body, trigger_message_id,
    validate,
};
use flow_like_types::inbox::{TRIGGER_TEXT_MAX_CHARS, TriggerBody, TriggerSource};
use serde_json::json;

fn message(text: &str) -> OutsideMessage {
    OutsideMessage {
        provider: ChannelProvider::Discord,
        chat_key: "guild:channel".into(),
        chat_title: "#general (Acme)".into(),
        message_id: "m1".into(),
        sender: OutsideSender {
            id: "4242".into(),
            name: "Ann".into(),
        },
        text: text.into(),
        attachments: Vec::new(),
        meta: json!({}),
    }
}

fn attachment(url: &str) -> OutsideAttachment {
    OutsideAttachment {
        name: "plan.pdf".into(),
        url: url.into(),
        content_type: Some("application/pdf".into()),
    }
}

#[test]
fn an_outside_person_gets_a_stable_id_that_is_no_sub_and_hides_the_provider_id() {
    let ann = outside_requester(ChannelProvider::Teams, "tenant:4242");
    assert!(ann.starts_with("ext:teams:"));
    assert!(!ann.contains("4242"));
    assert_eq!(
        ann,
        outside_requester(ChannelProvider::Teams, "tenant:4242")
    );
    assert_ne!(
        ann,
        outside_requester(ChannelProvider::Discord, "tenant:4242")
    );
    assert_ne!(
        ann,
        outside_requester(ChannelProvider::Teams, "tenant:4243")
    );
}

#[test]
fn a_chat_finds_its_link_again_and_messages_get_channel_trigger_ids() {
    let link = link_id("bot_1", ChannelProvider::Discord, "guild:channel");
    assert!(link.starts_with("chl_"));
    assert_eq!(
        link,
        link_id("bot_1", ChannelProvider::Discord, "guild:channel")
    );
    assert_ne!(
        link,
        link_id("bot_2", ChannelProvider::Discord, "guild:channel")
    );
    assert_ne!(
        link,
        link_id("bot_1", ChannelProvider::Teams, "guild:channel")
    );

    let trigger = trigger_message_id("bot_1", ChannelProvider::Discord, "guild:channel", "m1");
    assert!(trigger.starts_with(CHANNEL_MESSAGE_PREFIX));
    assert_ne!(
        trigger,
        trigger_message_id("bot_2", ChannelProvider::Discord, "guild:channel", "m1")
    );
}

#[test]
fn a_chat_message_becomes_an_untrusted_channel_trigger_naming_the_bot() {
    let mut chat = message("When is the release?");
    chat.attachments
        .push(attachment("https://cdn.example.com/plan.pdf"));
    let body = trigger_body("bot_1", "chl_x", &chat);
    assert_eq!(body.source, TriggerSource::Channel);
    assert_eq!(body.bot_id.as_deref(), Some("bot_1"));
    assert_eq!(body.ref_id, "chl_x");
    assert!(body.text.starts_with("Ann (Discord): When is the release?"));
    assert!(
        body.text
            .contains("[attachment: plan.pdf](https://cdn.example.com/plan.pdf)")
    );
    let payload = body.payload.clone().unwrap();
    assert_eq!(payload["untrusted"], true);
    assert_eq!(payload["owner"], false);
    assert_eq!(payload["provider"], "discord");
    assert_eq!(
        payload["sender"]["id"],
        outside_requester(ChannelProvider::Discord, "4242")
    );
    TriggerBody::parse(&serde_json::to_value(&body).unwrap()).unwrap();
}

#[test]
fn a_long_message_still_fits_a_trigger() {
    let body = trigger_body(
        "bot_1",
        "chl_x",
        &message(&"a".repeat(TRIGGER_TEXT_MAX_CHARS)),
    );
    assert!(body.text.chars().count() <= TRIGGER_TEXT_MAX_CHARS);
    TriggerBody::parse(&serde_json::to_value(&body).unwrap()).unwrap();
}

#[test]
fn a_nameless_sender_reads_as_someone() {
    let mut chat = message("hi");
    chat.sender.name = "  ".into();
    let body = trigger_body("bot_1", "chl_x", &chat);
    assert!(body.text.starts_with("Someone (Discord): hi"));
    assert_eq!(body.title, "Someone on Discord");
}

#[test]
fn a_chat_message_needs_content_https_links_and_small_reply_data() {
    assert!(validate(&message("hi")).is_ok());
    assert!(validate(&message("  ")).is_err());

    let mut only_file = message("");
    only_file
        .attachments
        .push(attachment("https://cdn.example.com/a.pdf"));
    assert!(validate(&only_file).is_ok());

    let mut plain_http = message("hi");
    plain_http
        .attachments
        .push(attachment("http://cdn.example.com/a.pdf"));
    assert!(validate(&plain_http).is_err());

    let mut many = message("hi");
    many.attachments = (0..11)
        .map(|_| attachment("https://cdn.example.com/a.pdf"))
        .collect();
    assert!(validate(&many).is_err());

    let mut big_meta = message("hi");
    big_meta.meta = json!({ "x": "a".repeat(5_000) });
    assert!(validate(&big_meta).is_err());

    let mut no_sender = message("hi");
    no_sender.sender.id = String::new();
    assert!(validate(&no_sender).is_err());
}

#[test]
fn a_discord_message_keys_its_chat_by_server_and_channel_or_as_a_dm() {
    let request = |guild: Option<&str>| -> DiscordMessageRequest {
        serde_json::from_value(json!({
            "chat_id": "c1",
            "guild_id": guild,
            "chat_title": "#general",
            "message_id": "m1",
            "sender": {"id": "4242", "name": "Ann"},
            "text": "hi",
        }))
        .unwrap()
    };
    assert_eq!(request(Some("g1")).into_message().chat_key, "g1:c1");
    assert_eq!(request(None).into_message().chat_key, "dm:c1");
    assert_eq!(request(Some(" ")).into_message().chat_key, "dm:c1");
    let message = request(Some("g1")).into_message();
    assert_eq!(message.provider, ChannelProvider::Discord);
    assert_eq!(message.meta["channel_id"], "c1");
}
