use super::{
    TeamsPermission,
    microsoft::{
        ChannelInfo, GraphChannel, GraphChat, GraphTeam, MeetingInfo, TeamInfo, TeamsMemberInfo,
        base_conversation,
    },
};
use serde_json::{Map, Value, json};

const MAX_NAME_CHARS: usize = 64;
const MAX_HTML_BYTES: usize = 24_000;

/// A trimmed, non-empty string at `pointer`.
pub(super) fn field<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn entities<'a>(activity: &'a Value, kind: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
    activity["entities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(move |entity| entity["type"] == kind)
}

pub(super) fn is_group(conversation_type: &str) -> bool {
    matches!(conversation_type, "groupChat" | "channel")
}

/// Whether the bot should answer: always in 1:1 chats, elsewhere only when @mentioned.
pub(super) fn addressed(conversation_type: &str, mentions_bot: bool) -> bool {
    conversation_type == "personal" || mentions_bot
}

/// Other bots never start runs, including bots Teams reports without a role.
pub(super) fn from_bot(activity: &Value) -> bool {
    activity["from"]["role"] == "bot"
        || (field(activity, "/from/aadObjectId").is_none()
            && field(activity, "/from/id").is_some_and(|id| id.starts_with("28:")))
}

fn is_bot_mention(activity: &Value, entity: &Value) -> bool {
    field(entity, "/mentioned/id").is_some_and(|id| Some(id) == field(activity, "/recipient/id"))
}

pub(super) fn mentions_bot(activity: &Value) -> bool {
    entities(activity, "mention").any(|entity| is_bot_mention(activity, entity))
}

/// The bot's name as the message @mentions it, from the mention's name or its `<at>` text.
pub(super) fn bot_mention_name(activity: &Value) -> Option<String> {
    entities(activity, "mention")
        .filter(|entity| is_bot_mention(activity, entity))
        .flat_map(|entity| [field(entity, "/mentioned/name"), field(entity, "/text")])
        .flatten()
        .map(|name| {
            clean_label(
                &name.replace("<at>", "").replace("</at>", ""),
                MAX_NAME_CHARS,
            )
        })
        .find(|name| !name.is_empty())
}

/// Whether the meeting's organizer is this user, by Bot Framework or Entra ID.
pub(super) fn organized_by(
    meeting: &MeetingInfo,
    user_id: Option<&str>,
    aad_object_id: Option<&str>,
) -> bool {
    meeting
        .organizer_id
        .as_deref()
        .is_some_and(|organizer| Some(organizer) == user_id)
        || meeting
            .organizer_aad_object_id
            .as_deref()
            .zip(aad_object_id)
            .is_some_and(|(organizer, user)| organizer.eq_ignore_ascii_case(user))
}

/// Everyone the message @mentions except the bot.
pub(super) fn mentions(activity: &Value) -> Vec<Value> {
    entities(activity, "mention")
        .filter(|entity| !is_bot_mention(activity, entity))
        .filter_map(|entity| {
            let mut mention = Map::new();
            set(&mut mention, "id", Some(field(entity, "/mentioned/id")?));
            set(
                &mut mention,
                "aad_object_id",
                field(entity, "/mentioned/aadObjectId"),
            );
            set(
                &mut mention,
                "name",
                field(entity, "/mentioned/name").map(|name| clean_label(name, MAX_NAME_CHARS)),
            );
            Some(Value::Object(mention))
        })
        .collect()
}

/// The message text without the bot's own mention, other `<at>X</at>` mentions as `@X`,
/// and without quoted-reply placeholders.
pub(super) fn clean_text(activity: &Value) -> String {
    let mut text = activity["text"].as_str().unwrap_or_default().to_owned();
    for entity in entities(activity, "mention").filter(|entity| is_bot_mention(activity, entity)) {
        if let Some(mention) = entity["text"].as_str().filter(|s| !s.is_empty()) {
            text = text.replace(mention, "");
        }
    }
    at_mentions(&without_quote_tags(&text)).trim().to_owned()
}

fn at_mentions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<at>") {
        let inner = &rest[start + 4..];
        let Some(end) = inner.find("</at>") else {
            break;
        };
        out.push_str(&rest[..start]);
        out.push('@');
        out.push_str(inner[..end].trim());
        rest = &inner[end + 5..];
    }
    out.push_str(rest);
    out
}

fn without_quote_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<quoted") {
        let tail = &rest[start + 7..];
        let Some(end) = tail
            .starts_with([' ', '/', '>'])
            .then(|| tail.find('>'))
            .flatten()
        else {
            out.push_str(&rest[..start + 7]);
            rest = tail;
            continue;
        };
        out.push_str(&rest[..start]);
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// One line of at most `max_chars`: control and invisible formatting characters removed,
/// whitespace collapsed to single spaces.
pub(super) fn clean_label(raw: &str, max_chars: usize) -> String {
    let visible: String = raw
        .chars()
        .filter(|&c| c.is_whitespace() || !(c.is_control() || invisible(c)))
        .collect();
    let collapsed = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .chars()
        .take(max_chars)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// The name history entries use for the sender.
pub(super) fn display_name(raw: Option<&str>, anonymous: bool) -> String {
    let name = clean_label(raw.unwrap_or_default(), MAX_NAME_CHARS);
    match (name.is_empty(), anonymous) {
        (false, _) => name,
        (true, true) => "Guest".into(),
        (true, false) => "Teams user".into(),
    }
}

/// Cuts on a char boundary, without a marker.
pub(super) fn truncate(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Cuts on a char boundary and ends with "…", staying within `max_bytes`.
pub(super) fn cap(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let kept = truncate(text, max_bytes.saturating_sub('…'.len_utf8()));
    format!("{kept}…")
}

/// The `quotedReply` entity Teams adds when the user replies to a specific message.
pub(super) fn quoted(activity: &Value) -> Option<Value> {
    let entity = entities(activity, "quotedReply").next()?;
    let quote = entity
        .get("quotedReply")
        .filter(|quote| quote.is_object())
        .unwrap_or(entity);
    let mut map = Map::new();
    set(&mut map, "message_id", field(quote, "/messageId"));
    set(&mut map, "sender_id", field(quote, "/senderId"));
    set(
        &mut map,
        "sender_name",
        field(quote, "/senderName").map(|name| clean_label(name, MAX_NAME_CHARS)),
    );
    set(&mut map, "preview", field(quote, "/preview"));
    set(&mut map, "time", field(quote, "/time"));
    (!map.is_empty()).then_some(Value::Object(map))
}

/// `> {sender}: {preview}` on one line, or `None` without a preview.
pub(super) fn quote_line(quoted: &Value) -> Option<String> {
    let preview = clean_label(quoted["preview"].as_str()?, usize::MAX);
    if preview.is_empty() {
        return None;
    }
    let sender = display_name(quoted["sender_name"].as_str(), false);
    Some(format!("> {sender}: {preview}"))
}

/// A user entry's history text: the quote first, then the message, and the author's name in
/// front when `author` is given (group chats and channels).
pub(super) fn history_content(author: Option<&str>, quote: Option<&str>, body: &str) -> String {
    let content = match quote {
        Some(quote) => format!("{quote}\n\n{body}").trim_end().to_owned(),
        None => body.to_owned(),
    };
    match author {
        Some(author) => format!("{author}: {content}"),
        None => content,
    }
}

/// A channel thread's root message: the ID after `;messageid=`, else the activity itself.
pub(super) fn thread_id(conversation_id: &str, activity_id: &str) -> String {
    conversation_id
        .split_once(";messageid=")
        .map(|(_, id)| id)
        .filter(|id| !id.is_empty())
        .unwrap_or(activity_id)
        .to_owned()
}

/// Where the message was posted. Fields are empty when Teams did not report them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Scope {
    pub conversation_type: String,
    pub team_id: String,
    pub channel_id: String,
    pub thread_id: String,
    pub meeting_id: String,
}

pub(super) fn scope(activity: &Value) -> Scope {
    let owned = |pointer: &str| field(activity, pointer).unwrap_or_default().to_owned();
    let conversation_type = owned("/conversation/conversationType");
    let conversation = field(activity, "/conversation/id").unwrap_or_default();
    let channel = conversation_type == "channel";
    let channel_id = match field(activity, "/channelData/channel/id") {
        Some(id) => id.to_owned(),
        None if channel => base_conversation(conversation).to_owned(),
        None => String::new(),
    };
    Scope {
        team_id: owned("/channelData/team/id"),
        channel_id,
        thread_id: if channel {
            thread_id(conversation, field(activity, "/id").unwrap_or_default())
        } else {
            String::new()
        },
        meeting_id: owned("/channelData/meeting/id"),
        conversation_type,
    }
}

/// The `text/html` rendering Teams attaches to formatted messages.
pub(super) fn html(activity: &Value) -> Option<String> {
    activity["attachments"]
        .as_array()?
        .iter()
        .filter(|attachment| {
            attachment["contentType"]
                .as_str()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("text/html"))
        })
        .find_map(|attachment| attachment["content"].as_str())
        .map(|html| truncate(html, MAX_HTML_BYTES).to_owned())
        .filter(|html| !html.trim().is_empty())
}

fn set(map: &mut Map<String, Value>, key: &str, value: Option<impl Into<String>>) {
    if let Some(value) = value.map(Into::into).filter(|value| !value.is_empty()) {
        map.insert(key.into(), Value::String(value));
    }
}

fn fill(map: &mut Map<String, Value>, key: &str, value: Option<impl Into<String>>) {
    if !map.contains_key(key) {
        set(map, key, value);
    }
}

/// Microsoft lookups that complete the activity's own context. Every part is optional.
#[derive(Default)]
pub(super) struct Enrichment {
    pub member: Option<TeamsMemberInfo>,
    pub team: Option<TeamInfo>,
    pub channels: Option<Vec<ChannelInfo>>,
    pub meeting: Option<MeetingInfo>,
    pub meeting_role: Option<String>,
    pub graph_team: Option<GraphTeam>,
    pub graph_channel: Option<GraphChannel>,
    pub graph_chat: Option<GraphChat>,
}

/// `local_session.teams`: missing values are omitted, never `null`.
#[derive(Clone, Debug, Default)]
pub(super) struct Context {
    session_id: String,
    permissions: Vec<TeamsPermission>,
    conversation: Map<String, Value>,
    user: Map<String, Value>,
    team: Map<String, Value>,
    channel: Map<String, Value>,
    meeting: Map<String, Value>,
    message: Map<String, Value>,
}

impl Context {
    pub(super) fn from_activity(
        activity: &Value,
        session_id: &str,
        permissions: &[TeamsPermission],
        anonymous: bool,
    ) -> Self {
        let scope = scope(activity);
        let at = |pointer: &str| field(activity, pointer);
        let client = entities(activity, "clientInfo").next();
        let client_at = |pointer: &str| client.and_then(|client| field(client, pointer));

        let mut conversation = Map::new();
        set(&mut conversation, "id", at("/conversation/id"));
        set(
            &mut conversation,
            "type",
            Some(scope.conversation_type.as_str()),
        );
        set(&mut conversation, "name", at("/conversation/name"));
        set(
            &mut conversation,
            "tenant_id",
            at("/conversation/tenantId").or_else(|| at("/channelData/tenant/id")),
        );
        set(
            &mut conversation,
            "thread_id",
            Some(scope.thread_id.as_str()),
        );
        conversation.insert(
            "is_group".into(),
            Value::Bool(
                activity["conversation"]["isGroup"]
                    .as_bool()
                    .unwrap_or_else(|| is_group(&scope.conversation_type)),
            ),
        );

        let mut user = Map::new();
        set(&mut user, "id", at("/from/id"));
        set(&mut user, "aad_object_id", at("/from/aadObjectId"));
        set(
            &mut user,
            "name",
            at("/from/name").map(|name| clean_label(name, MAX_NAME_CHARS)),
        );
        if anonymous {
            set(&mut user, "role", Some("anonymous"));
        }
        user.insert("anonymous".into(), Value::Bool(anonymous));
        set(
            &mut user,
            "locale",
            client_at("/locale").or_else(|| at("/locale")),
        );
        set(
            &mut user,
            "timezone",
            at("/localTimezone").or_else(|| client_at("/timezone")),
        );
        set(&mut user, "platform", client_at("/platform"));
        set(&mut user, "country", client_at("/country"));

        let mut team = Map::new();
        if !scope.team_id.is_empty() {
            set(&mut team, "id", Some(scope.team_id.as_str()));
            set(&mut team, "graph_id", at("/channelData/team/aadGroupId"));
            set(&mut team, "name", at("/channelData/team/name"));
        }
        let mut channel = Map::new();
        if !scope.channel_id.is_empty() {
            set(&mut channel, "id", Some(scope.channel_id.as_str()));
            set(&mut channel, "name", at("/channelData/channel/name"));
        }
        let mut meeting = Map::new();
        set(&mut meeting, "id", Some(scope.meeting_id.as_str()));

        let mut message = Map::new();
        set(&mut message, "id", at("/id"));
        set(&mut message, "timestamp", at("/timestamp"));
        set(&mut message, "local_timestamp", at("/localTimestamp"));
        set(&mut message, "text", Some(clean_text(activity)));
        set(&mut message, "html", html(activity));
        set(&mut message, "reply_to_id", at("/replyToId"));
        message.insert("mentions_bot".into(), Value::Bool(mentions_bot(activity)));
        message.insert("mentions".into(), Value::Array(mentions(activity)));
        if let Some(quote) = quoted(activity) {
            message.insert("quoted".into(), quote);
        }

        Self {
            session_id: session_id.to_owned(),
            permissions: permissions.to_vec(),
            conversation,
            user,
            team,
            channel,
            meeting,
            message,
        }
    }

    /// The team's Microsoft 365 group, needed for Graph lookups.
    pub(super) fn graph_group(&self) -> Option<&str> {
        self.team.get("graph_id").and_then(Value::as_str)
    }

    /// Adds lookup results without overwriting what the activity reported.
    pub(super) fn enrich(&mut self, enrichment: Enrichment) {
        if let Some(member) = enrichment.member {
            let user = &mut self.user;
            fill(user, "aad_object_id", member.aad_object_id);
            fill(
                user,
                "name",
                member
                    .name
                    .as_deref()
                    .map(|name| clean_label(name, MAX_NAME_CHARS)),
            );
            fill(user, "given_name", member.given_name);
            fill(user, "surname", member.surname);
            fill(user, "email", member.email);
            fill(user, "user_principal_name", member.user_principal_name);
            fill(user, "role", member.user_role);
            if let (Some(home), Some(conversation)) = (
                member.tenant_id.as_deref(),
                self.conversation.get("tenant_id").and_then(Value::as_str),
            ) {
                user.insert("external".into(), Value::Bool(home != conversation));
            }
            fill(user, "tenant_id", member.tenant_id);
        }
        if let Some(role) = enrichment.meeting_role {
            fill(&mut self.user, "meeting_role", Some(role));
        }
        if let Some(team) = enrichment.team {
            fill(&mut self.team, "id", Some(team.id));
            fill(&mut self.team, "graph_id", team.aad_group_id);
            fill(&mut self.team, "name", team.name);
        }
        if let (Some(channels), Some(id)) = (
            enrichment.channels,
            self.channel.get("id").and_then(Value::as_str),
        ) {
            let name = channels
                .into_iter()
                .find(|channel| channel.id == id)
                .map(|channel| channel.name);
            fill(&mut self.channel, "name", name);
        }
        if let Some(meeting) = enrichment.meeting {
            let organizer = meeting.organizer_name.clone().or_else(|| {
                let user = |key: &str| self.user.get(key).and_then(Value::as_str);
                organized_by(&meeting, user("id"), user("aad_object_id"))
                    .then(|| user("name").map(str::to_owned))
                    .flatten()
            });
            fill(&mut self.meeting, "title", meeting.title);
            fill(&mut self.meeting, "start", meeting.start);
            fill(&mut self.meeting, "end", meeting.end);
            fill(&mut self.meeting, "join_url", meeting.join_url);
            fill(
                &mut self.meeting,
                "organizer",
                organizer.map(|name| clean_label(&name, MAX_NAME_CHARS)),
            );
        }
        if let Some(team) = enrichment.graph_team {
            fill(&mut self.team, "description", team.description);
            fill(&mut self.team, "web_url", team.web_url);
            fill(&mut self.team, "visibility", team.visibility);
        }
        if let Some(channel) = enrichment.graph_channel {
            fill(&mut self.channel, "description", channel.description);
            fill(
                &mut self.channel,
                "membership_type",
                channel.membership_type,
            );
            fill(&mut self.channel, "web_url", channel.web_url);
        }
        if let Some(chat) = enrichment.graph_chat {
            fill(&mut self.conversation, "topic", chat.topic);
            fill(&mut self.conversation, "web_url", chat.web_url);
        }
    }

    pub(super) fn set_files(&mut self, files: Vec<Value>) {
        self.message.insert("files".into(), Value::Array(files));
    }

    pub(super) fn into_value(self) -> Value {
        let mut context = json!({"session_id": self.session_id, "permissions": self.permissions});
        for (key, section) in [
            ("conversation", self.conversation),
            ("user", self.user),
            ("team", self.team),
            ("channel", self.channel),
            ("meeting", self.meeting),
            ("message", self.message),
        ] {
            if !section.is_empty() {
                context[key] = Value::Object(section);
            }
        }
        context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOT: &str = "28:bot-app-id";

    fn channel_activity() -> Value {
        json!({
            "type": "message",
            "id": "1727000000001",
            "timestamp": "2026-09-28T09:00:00.000Z",
            "localTimestamp": "2026-09-28T11:00:00.000+02:00",
            "localTimezone": "Europe/Berlin",
            "locale": "en-US",
            "replyToId": "1727000000000",
            "text": "<at>Flow Bot</at> summarize what <at>Anna Smith</at> said ",
            "from": {"id": "29:felix", "aadObjectId": "9d3e08f9-a7ae-43aa-a4d3-de3f319a8a9c", "name": " Felix\n  Schultz "},
            "recipient": {"id": BOT, "name": "Flow Bot"},
            "conversation": {
                "id": "19:general@thread.tacv2;messageid=1727000000000",
                "conversationType": "channel",
                "tenantId": "tenant",
                "isGroup": true
            },
            "channelData": {
                "team": {"id": "19:team@thread.tacv2", "aadGroupId": "group-1"},
                "channel": {"id": "19:general@thread.tacv2"},
                "tenant": {"id": "tenant"}
            },
            "entities": [
                {"type": "mention", "text": "<at>Flow Bot</at>", "mentioned": {"id": BOT, "name": "Flow Bot"}},
                {"type": "mention", "text": "<at>Anna Smith</at>", "mentioned": {"id": "29:anna", "name": "Anna Smith"}},
                {"type": "clientInfo", "locale": "de-DE", "country": "DE", "platform": "Windows", "timezone": "America/New_York"},
                {"type": "quotedReply", "quotedReply": {"messageId": "1726", "senderId": "29:anna", "senderName": "Anna Smith", "preview": "Budget is\napproved", "time": "2026-09-28T08:00:00Z"}}
            ],
            "attachments": [
                {"contentType": "text/html", "content": "<p><at>Flow Bot</at> summarize</p>"}
            ]
        })
    }

    #[test]
    fn text_drops_the_bot_mention_and_keeps_other_people_readable() {
        let activity = channel_activity();
        assert_eq!(clean_text(&activity), "summarize what @Anna Smith said");
        assert!(mentions_bot(&activity));
        assert_eq!(
            mentions(&activity),
            vec![json!({"id":"29:anna","name":"Anna Smith"})]
        );
        let quoted = json!({
            "text": "<quoted messageId=\"1726\"/>Agreed, <at>Bot</at>",
            "recipient": {"id": BOT},
            "entities": [{"type":"mention","text":"<at>Bot</at>","mentioned":{"id":BOT}}]
        });
        assert_eq!(clean_text(&quoted), "Agreed,");
        assert_eq!(
            at_mentions("<atx> <at>A</at> and <at> B </at> <at>open"),
            "<atx> @A and @B <at>open"
        );
        assert_eq!(without_quote_tags("<quotedx> <quoted"), "<quotedx> <quoted");
    }

    #[test]
    fn mentions_of_other_bots_or_people_do_not_address_the_bot() {
        let activity = json!({
            "text": "<at>Other</at> hi",
            "recipient": {"id": BOT},
            "entities": [{"type":"mention","text":"<at>Other</at>","mentioned":{"id":"28:other"}}]
        });
        assert!(!mentions_bot(&activity));
        assert!(addressed("personal", false));
        assert!(addressed("channel", true));
        assert!(!addressed("groupChat", false));
        assert!(!addressed("", false));
    }

    #[test]
    fn bots_are_recognized_with_or_without_a_role() {
        assert!(from_bot(&json!({"from":{"id":"29:x","role":"bot"}})));
        assert!(from_bot(&json!({"from":{"id":"28:other-bot"}})));
        assert!(!from_bot(
            &json!({"from":{"id":"28:odd","aadObjectId":"oid"}})
        ));
        assert!(!from_bot(&json!({"from":{"id":"29:guest"}})));
    }

    #[test]
    fn display_names_are_one_short_line() {
        assert_eq!(
            display_name(Some(" Felix\n\t Schultz "), false),
            "Felix Schultz"
        );
        assert_eq!(
            display_name(Some("Fe\u{0}lix\u{202e}\u{200b} S"), false),
            "Felix S"
        );
        assert_eq!(display_name(Some("  "), false), "Teams user");
        assert_eq!(display_name(None, true), "Guest");
        assert_eq!(display_name(Some("Visitor"), true), "Visitor");
        let long = display_name(Some(&"é".repeat(100)), false);
        assert_eq!(long.chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn history_text_names_the_author_in_groups_and_quotes_first() {
        let quote = quoted(&channel_activity()).unwrap();
        assert_eq!(
            quote,
            json!({"message_id":"1726","sender_id":"29:anna","sender_name":"Anna Smith","preview":"Budget is\napproved","time":"2026-09-28T08:00:00Z"})
        );
        let line = quote_line(&quote).unwrap();
        assert_eq!(line, "> Anna Smith: Budget is approved");
        assert_eq!(
            history_content(Some("Felix"), Some(&line), "Thanks"),
            "Felix: > Anna Smith: Budget is approved\n\nThanks"
        );
        assert_eq!(history_content(None, None, "hi"), "hi");
        assert_eq!(
            history_content(Some("Felix"), Some("> A: b"), ""),
            "Felix: > A: b"
        );
        assert_eq!(quote_line(&json!({"sender_name":"A"})), None);
        assert_eq!(
            quote_line(&json!({"preview":"x"})).as_deref(),
            Some("> Teams user: x")
        );
    }

    #[test]
    fn caps_keep_char_boundaries() {
        assert_eq!(truncate("aé", 2), "a");
        assert_eq!(cap("short", 10), "short");
        let capped = cap(&"é".repeat(10), 8);
        assert_eq!(capped, "éé…");
        assert!(capped.len() <= 8);
    }

    #[test]
    fn scopes_follow_the_conversation_type() {
        assert_eq!(
            scope(&channel_activity()),
            Scope {
                conversation_type: "channel".into(),
                team_id: "19:team@thread.tacv2".into(),
                channel_id: "19:general@thread.tacv2".into(),
                thread_id: "1727000000000".into(),
                meeting_id: String::new(),
            }
        );
        let root = scope(&json!({
            "id": "17",
            "conversation": {"id": "19:c@thread.tacv2", "conversationType": "channel"}
        }));
        assert_eq!(root.channel_id, "19:c@thread.tacv2");
        assert_eq!(root.thread_id, "17");
        let meeting = scope(&json!({
            "id": "5",
            "conversation": {"id": "19:meeting_x@thread.v2", "conversationType": "groupChat"},
            "channelData": {"meeting": {"id": "MCMx"}}
        }));
        assert_eq!(meeting.meeting_id, "MCMx");
        assert_eq!(meeting.thread_id, "");
        assert_eq!(meeting.channel_id, "");
        assert_eq!(thread_id("a;messageid=", "fallback"), "fallback");
    }

    #[test]
    fn context_reads_the_activity_and_omits_what_is_missing() {
        let context = Context::from_activity(
            &channel_activity(),
            "run",
            &[TeamsPermission::ReadMessages],
            false,
        )
        .into_value();
        assert_eq!(context["session_id"], "run");
        assert_eq!(context["permissions"], json!(["read_messages"]));
        assert_eq!(
            context["conversation"],
            json!({
                "id": "19:general@thread.tacv2;messageid=1727000000000",
                "type": "channel",
                "tenant_id": "tenant",
                "thread_id": "1727000000000",
                "is_group": true
            })
        );
        assert_eq!(
            context["user"],
            json!({
                "id": "29:felix",
                "aad_object_id": "9d3e08f9-a7ae-43aa-a4d3-de3f319a8a9c",
                "name": "Felix Schultz",
                "anonymous": false,
                "locale": "de-DE",
                "timezone": "Europe/Berlin",
                "platform": "Windows",
                "country": "DE"
            })
        );
        assert_eq!(
            context["team"],
            json!({"id":"19:team@thread.tacv2","graph_id":"group-1"})
        );
        assert_eq!(context["channel"], json!({"id":"19:general@thread.tacv2"}));
        assert!(context.get("meeting").is_none());
        let message = &context["message"];
        assert_eq!(message["text"], "summarize what @Anna Smith said");
        assert_eq!(message["html"], "<p><at>Flow Bot</at> summarize</p>");
        assert_eq!(message["reply_to_id"], "1727000000000");
        assert_eq!(message["local_timestamp"], "2026-09-28T11:00:00.000+02:00");
        assert_eq!(message["mentions_bot"], true);
        assert_eq!(message["quoted"]["sender_name"], "Anna Smith");
        assert!(message.get("files").is_none());
        assert!(!context.to_string().contains("null"));
    }

    #[test]
    fn anonymous_guests_and_personal_chats_carry_what_teams_sends() {
        let context = Context::from_activity(
            &json!({
                "id": "1",
                "text": "hello",
                "locale": "fr-FR",
                "from": {"id": "29:guest"},
                "recipient": {"id": BOT},
                "conversation": {"id": "a:1", "conversationType": "personal"},
                "entities": [{"type":"clientInfo","timezone":"Europe/Paris"}]
            }),
            "run",
            &[],
            true,
        )
        .into_value();
        assert_eq!(
            context["user"],
            json!({"id":"29:guest","role":"anonymous","anonymous":true,"locale":"fr-FR","timezone":"Europe/Paris"})
        );
        assert_eq!(context["conversation"]["is_group"], false);
        assert_eq!(context["permissions"], json!([]));
        assert_eq!(context["message"]["mentions"], json!([]));
    }

    #[test]
    fn html_is_capped_and_only_taken_from_html_attachments() {
        let big = format!("<p>{}</p>", "é".repeat(20_000));
        let html = html(&json!({"attachments":[
            {"contentType":"image/png","content":"x"},
            {"contentType":"Text/HTML","content":big}
        ]}))
        .unwrap();
        assert!(html.len() <= MAX_HTML_BYTES);
        assert!(html.starts_with("<p>é"));
        assert_eq!(
            super::html(&json!({"attachments":[{"contentType":"text/html","content":" "}]})),
            None
        );
    }

    #[test]
    fn lookups_fill_gaps_without_overwriting_the_activity() {
        let mut context = Context::from_activity(&channel_activity(), "run", &[], false);
        assert_eq!(context.graph_group(), Some("group-1"));
        context.enrich(Enrichment {
            member: Some(TeamsMemberInfo {
                id: "29:felix".into(),
                name: Some("Other Name".into()),
                given_name: Some("Felix".into()),
                email: Some("felix@example.com".into()),
                tenant_id: Some("home".into()),
                user_role: Some("user".into()),
                ..Default::default()
            }),
            team: Some(TeamInfo {
                id: "19:team@thread.tacv2".into(),
                name: Some("Support".into()),
                aad_group_id: Some("ignored".into()),
            }),
            channels: Some(vec![
                ChannelInfo {
                    id: "19:other@thread.tacv2".into(),
                    name: "Other".into(),
                },
                ChannelInfo {
                    id: "19:general@thread.tacv2".into(),
                    name: "General".into(),
                },
            ]),
            meeting_role: Some("Organizer".into()),
            graph_team: Some(GraphTeam {
                description: Some("Support team".into()),
                web_url: Some("https://teams.microsoft.com/team".into()),
                visibility: Some("private".into()),
            }),
            graph_channel: Some(GraphChannel {
                description: None,
                membership_type: Some("standard".into()),
                web_url: Some("https://teams.microsoft.com/channel".into()),
            }),
            graph_chat: Some(GraphChat {
                topic: Some("Launch".into()),
                chat_type: Some("group".into()),
                web_url: None,
            }),
            meeting: Some(MeetingInfo {
                title: Some("All Hands".into()),
                organizer_name: Some("Anna".into()),
                ..Default::default()
            }),
        });
        let context = context.into_value();
        let user = &context["user"];
        assert_eq!(user["name"], "Felix Schultz");
        assert_eq!(user["given_name"], "Felix");
        assert_eq!(user["email"], "felix@example.com");
        assert_eq!(user["tenant_id"], "home");
        assert_eq!(user["role"], "user");
        assert_eq!(user["external"], true);
        assert_eq!(user["meeting_role"], "Organizer");
        assert_eq!(
            context["team"],
            json!({"id":"19:team@thread.tacv2","graph_id":"group-1","name":"Support","description":"Support team","web_url":"https://teams.microsoft.com/team","visibility":"private"})
        );
        assert_eq!(
            context["channel"],
            json!({"id":"19:general@thread.tacv2","name":"General","membership_type":"standard","web_url":"https://teams.microsoft.com/channel"})
        );
        assert_eq!(context["conversation"]["topic"], "Launch");
        assert_eq!(
            context["meeting"],
            json!({"title":"All Hands","organizer":"Anna"})
        );
    }

    #[test]
    fn the_bot_mention_name_comes_from_the_entity_or_its_at_text() {
        assert_eq!(
            bot_mention_name(&channel_activity()).as_deref(),
            Some("Flow Bot")
        );
        let at_text_only = json!({
            "recipient": {"id": BOT},
            "entities": [
                {"type":"mention","text":"<at>Anna</at>","mentioned":{"id":"29:anna","name":"Anna"}},
                {"type":"mention","text":"<at> Flow\nBot </at>","mentioned":{"id":BOT}}
            ]
        });
        assert_eq!(bot_mention_name(&at_text_only).as_deref(), Some("Flow Bot"));
        let unnamed = json!({
            "recipient": {"id": BOT},
            "entities": [{"type":"mention","text":"<at></at>","mentioned":{"id":BOT}}]
        });
        assert_eq!(bot_mention_name(&unnamed), None);
        assert_eq!(bot_mention_name(&json!({"recipient":{"id":BOT}})), None);
    }

    #[test]
    fn a_meeting_organized_by_the_sender_is_named_after_them() {
        let organizer = |id: Option<&str>, aad: Option<&str>| MeetingInfo {
            title: Some("Standup".into()),
            organizer_id: id.map(Into::into),
            organizer_aad_object_id: aad.map(Into::into),
            ..Default::default()
        };
        let sender = organizer(Some("29:felix"), None);
        assert!(organized_by(&sender, Some("29:felix"), None));
        let by_oid = organizer(None, Some("9D3E08F9-A7AE-43AA-A4D3-DE3F319A8A9C"));
        assert!(organized_by(
            &by_oid,
            Some("29:other"),
            Some("9d3e08f9-a7ae-43aa-a4d3-de3f319a8a9c")
        ));
        assert!(!organized_by(&organizer(None, None), None, None));
        assert!(!organized_by(
            &organizer(Some("29:anna"), None),
            Some("29:felix"),
            None
        ));

        let mut context = Context::from_activity(&channel_activity(), "run", &[], false);
        context.enrich(Enrichment {
            meeting: Some(sender),
            ..Default::default()
        });
        assert_eq!(
            context.into_value()["meeting"]["organizer"],
            "Felix Schultz"
        );
        let mut context = Context::from_activity(&channel_activity(), "run", &[], false);
        context.enrich(Enrichment {
            meeting: Some(organizer(Some("29:anna"), None)),
            ..Default::default()
        });
        assert!(context.into_value()["meeting"].get("organizer").is_none());
    }

    #[test]
    fn members_from_the_conversation_tenant_are_not_external() {
        let mut context = Context::from_activity(&channel_activity(), "run", &[], false);
        context.enrich(Enrichment {
            member: Some(TeamsMemberInfo {
                id: "29:felix".into(),
                tenant_id: Some("tenant".into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        context.set_files(vec![json!({"name":"a.png"})]);
        let context = context.into_value();
        assert_eq!(context["user"]["external"], false);
        assert_eq!(context["message"]["files"], json!([{"name":"a.png"}]));
    }
}
