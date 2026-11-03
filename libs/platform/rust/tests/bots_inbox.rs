use flow_like_platform::{
    Auth, BotCapability, BotClassification, BotLocation, BotModelPolicy, BotSecretEncryption,
    Client, ConversationEncryption, CreateBot, CreateConversation, DataClassification, Error,
    InboxEntriesQuery, InboxEntry, InboxEntryKind, ReactionOp, RegisterInboxKey, SendInboxEntry,
    SetBotSecret, UpdateBot, Value, json, new_inbox_message_id, plain_envelope,
};
use futures_util::{StreamExt, TryStreamExt};
use reqwest::Url;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};

const NOW: &str = "2026-11-02T10:00:00.000Z";

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Option<Value>,
}

fn ok(body: Value) -> Reply {
    Reply {
        status: 200,
        body: Some(body),
    }
}

fn no_content() -> Reply {
    Reply {
        status: 204,
        body: None,
    }
}

fn failure(status: u16, code: &str, message: &str) -> Reply {
    Reply {
        status,
        body: Some(json!({ "error": { "code": code, "message": message } })),
    }
}

#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    query: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
    body: Option<Value>,
}

impl Request {
    fn route(&self) -> (&str, &str) {
        (&self.method, &self.path)
    }
}

/// Answers one connection per reply, in order, and hands every request to the test.
struct Server {
    url: String,
    requests: mpsc::Receiver<Request>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Request {
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let read = socket.read(&mut chunk).await.unwrap();
        assert_ne!(read, 0, "connection closed before the request head");
        data.extend_from_slice(&chunk[..read]);
        if let Some(end) = data.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let head = String::from_utf8(data[..head_end].to_vec()).unwrap();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap().split_whitespace();
    let method = first.next().unwrap().to_owned();
    let target = first.next().unwrap().to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let length = headers
        .get("content-length")
        .map_or(0, |value| value.parse::<usize>().unwrap());
    while data.len() < head_end + length {
        let read = socket.read(&mut chunk).await.unwrap();
        assert_ne!(read, 0, "connection closed before the request body");
        data.extend_from_slice(&chunk[..read]);
    }
    let url = Url::parse(&format!("http://server{target}")).unwrap();
    let body = &data[head_end..head_end + length];
    Request {
        method,
        path: url.path().to_owned(),
        query: url.query_pairs().into_owned().collect(),
        headers,
        body: (!body.is_empty()).then(|| serde_json::from_slice(body).unwrap()),
    }
}

impl Server {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (sender, requests) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                sender.send(request).await.unwrap();
                let body = reply
                    .body
                    .map(|body| serde_json::to_vec(&body).unwrap())
                    .unwrap_or_default();
                let head = format!(
                    "HTTP/1.1 {} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.status,
                    body.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }

    fn client(&self) -> Client {
        Client::new(&self.url, Auth::PersonalAccessToken("pat_owner".into())).unwrap()
    }

    async fn take(&mut self) -> Request {
        tokio::time::timeout(Duration::from_secs(2), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
}

fn bot() -> Value {
    json!({
        "id": "bot_1",
        "owner_id": "user_1",
        "system": null,
        "preset_id": null,
        "name": "Researcher",
        "description": null,
        "instructions": null,
        "appearance": { "kind": "pip" },
        "model": { "primary": "bit_model", "fallbacks": ["bit_fallback"] },
        "classification": { "classification": "internal", "pii": true },
        "capabilities": ["internet", "telepathy"],
        "profile_id": null,
        "location": "server",
        "device_id": null,
        "import_key": null,
        "created_at": NOW,
        "updated_at": NOW,
    })
}

fn entry(id: &str) -> Value {
    json!({
        "id": id,
        "message_id": format!("m_{id}"),
        "conversation_id": "conv",
        "sender_id": "bot_1",
        "kind": "message",
        "envelope": { "v": 1, "enc": "none", "body": { "text": id } },
        "created_at": NOW,
        "cursor": format!("1700000000000000:{id}"),
    })
}

fn conversation() -> Value {
    json!({
        "id": "conv",
        "kind": "direct",
        "title": null,
        "encryption": "plain",
        "created_by": "user_1",
        "created_at": NOW,
        "last_activity_at": NOW,
        "members": [
            { "principal_id": "user_1", "kind": "user", "added_by": null, "joined_at": NOW },
            { "principal_id": "bot_1", "kind": "bot", "added_by": "user_1", "joined_at": NOW },
        ],
        "unread_count": 2,
        "last_entry": entry("last"),
    })
}

fn query(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn is_message_id(value: &Value) -> bool {
    value.as_str().is_some_and(|id| {
        id.len() == 36
            && id.starts_with("msg_")
            && id[4..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[tokio::test]
async fn bots_are_listed_created_and_updated_with_typed_models() {
    let mut server = Server::start(vec![
        ok(json!({ "bots": [bot()] })),
        ok(bot()),
        ok(bot()),
        ok(bot()),
    ])
    .await;
    let client = server.client();

    let bots = client.list_bots().await.unwrap();
    let request = server.take().await;
    assert_eq!(request.route(), ("GET", "/api/v1/bots"));
    assert_eq!(request.headers["authorization"], "pat_owner");
    assert!(!request.headers.contains_key("x-api-key"));
    assert_eq!(bots[0].model.fallbacks, ["bit_fallback"]);
    assert_eq!(
        bots[0].classification.classification,
        Some(DataClassification::Internal)
    );
    assert_eq!(
        bots[0].capabilities,
        [
            BotCapability::Internet,
            BotCapability::Other("telepathy".into())
        ]
    );
    assert_eq!(bots[0].location, BotLocation::Server);

    client
        .create_bot(&CreateBot {
            capabilities: vec![BotCapability::Internet, BotCapability::SelectedApps],
            model: Some(BotModelPolicy {
                primary: Some("bit_model".into()),
                fallbacks: Vec::new(),
            }),
            classification: Some(BotClassification {
                classification: None,
                pii: true,
            }),
            ..CreateBot::new("Researcher")
        })
        .await
        .unwrap();
    let request = server.take().await;
    assert_eq!(request.route(), ("POST", "/api/v1/bots"));
    assert_eq!(
        request.body,
        Some(json!({
            "name": "Researcher",
            "appearance": { "kind": "pip" },
            "capabilities": ["internet", "selected_apps"],
            "model": { "primary": "bit_model", "fallbacks": [] },
            "classification": { "pii": true },
        }))
    );

    client
        .create_bot(&CreateBot {
            appearance: json!({ "kind": "robo" }),
            location: Some(BotLocation::Desktop),
            ..CreateBot::new("Desk")
        })
        .await
        .unwrap();
    assert_eq!(
        server.take().await.body,
        Some(json!({ "name": "Desk", "appearance": { "kind": "robo" }, "location": "desktop" }))
    );

    client
        .update_bot(
            "bot /1",
            &UpdateBot {
                description: Some(None),
                instructions: Some(Some("Be brief".into())),
                capabilities: Some(Vec::new()),
                ..UpdateBot::default()
            },
        )
        .await
        .unwrap();
    let request = server.take().await;
    assert_eq!(request.route(), ("PATCH", "/api/v1/bots/bot%20%2F1"));
    assert_eq!(
        request.body,
        Some(json!({ "description": null, "instructions": "Be brief", "capabilities": [] }))
    );
}

#[tokio::test]
async fn bot_apps_and_secrets_use_their_routes() {
    let secret = json!({
        "name": "API_TOKEN",
        "encryption": "server",
        "description": "CRM",
        "created_at": NOW,
        "updated_at": NOW,
        "last_used_at": null,
    });
    let mut server = Server::start(vec![
        ok(bot()),
        no_content(),
        ok(json!({ "apps": [{ "app_id": "app", "added_at": NOW, "owner_role": null }] })),
        no_content(),
        no_content(),
        ok(json!({ "secrets": [secret.clone()] })),
        ok(secret.clone()),
        ok(secret.clone()),
        no_content(),
    ])
    .await;
    let client = server.client();

    assert_eq!(client.get_bot("bot_1").await.unwrap().id, "bot_1");
    assert_eq!(server.take().await.route(), ("GET", "/api/v1/bots/bot_1"));

    client.delete_bot("bot_1").await.unwrap();
    assert_eq!(
        server.take().await.route(),
        ("DELETE", "/api/v1/bots/bot_1")
    );

    let apps = client.list_bot_apps("bot_1").await.unwrap();
    assert_eq!(
        (apps[0].app_id.as_str(), apps[0].owner_role.as_deref()),
        ("app", None)
    );
    assert_eq!(
        server.take().await.route(),
        ("GET", "/api/v1/bots/bot_1/apps")
    );

    client.attach_bot_app("bot_1", "app/1").await.unwrap();
    let request = server.take().await;
    assert_eq!(request.route(), ("PUT", "/api/v1/bots/bot_1/apps/app%2F1"));
    assert_eq!(request.body, None);

    client.detach_bot_app("bot_1", "app").await.unwrap();
    assert_eq!(
        server.take().await.route(),
        ("DELETE", "/api/v1/bots/bot_1/apps/app")
    );

    let secrets = client.list_bot_secrets("bot_1").await.unwrap();
    assert_eq!(secrets[0].encryption, BotSecretEncryption::Server);
    assert_eq!(
        server.take().await.route(),
        ("GET", "/api/v1/bots/bot_1/secrets")
    );

    let value = SetBotSecret::value("s3cr3t").description("CRM");
    assert!(!format!("{value:?}").contains("s3cr3t"));
    let stored = client
        .set_bot_secret("bot_1", "API_TOKEN", &value)
        .await
        .unwrap();
    assert_eq!(stored.description.as_deref(), Some("CRM"));
    let request = server.take().await;
    assert_eq!(
        request.route(),
        ("PUT", "/api/v1/bots/bot_1/secrets/API_TOKEN")
    );
    assert_eq!(
        request.body,
        Some(json!({ "value": "s3cr3t", "description": "CRM" }))
    );

    let sealed = json!({ "v": 1, "enc": "hpke", "key_id": "k", "enc_key": "e", "ct": "c" });
    client
        .set_bot_secret(
            "bot_1",
            "API_TOKEN",
            &SetBotSecret::sealed(sealed.clone()).clear_description(),
        )
        .await
        .unwrap();
    assert_eq!(
        server.take().await.body,
        Some(json!({ "sealed": sealed, "description": null }))
    );

    client
        .delete_bot_secret("bot_1", "API_TOKEN")
        .await
        .unwrap();
    assert_eq!(
        server.take().await.route(),
        ("DELETE", "/api/v1/bots/bot_1/secrets/API_TOKEN")
    );
}

#[tokio::test]
async fn errors_keep_the_status_and_the_server_code() {
    let mut server = Server::start(vec![
        failure(409, "BOT_LIMIT_REACHED", "You have 50 bots"),
        failure(404, "NOT_FOUND", "Bot 'x' not found"),
        failure(
            403,
            "FORBIDDEN",
            "APIKey user is not allowed on this endpoint",
        ),
    ])
    .await;
    let client = server.client();

    let error = client
        .create_bot(&CreateBot::new("One more"))
        .await
        .unwrap_err();
    let Error::Api { status, body, .. } = &error else {
        panic!("expected an API error, got {error:?}");
    };
    assert_eq!(*status, 409);
    assert_eq!(body["error"]["code"], "BOT_LIMIT_REACHED");
    assert!(client.get_bot("x").await.unwrap_err().is_not_found());
    assert!(client.list_conversations(None).await.unwrap_err().is_auth());
    for _ in 0..3 {
        server.take().await;
    }
    assert!(matches!(
        client.get_bot("..").await,
        Err(Error::Configuration(_))
    ));
}

#[tokio::test]
async fn conversations_are_read_as_yourself_or_your_bot_and_managed() {
    let mut server = Server::start(vec![
        ok(json!({ "conversations": [conversation()] })),
        ok(conversation()),
        ok(conversation()),
        ok(conversation()),
        ok(conversation()),
        ok(conversation()),
        no_content(),
    ])
    .await;
    let client = server.client();

    let conversations = client.list_conversations(Some("bot_1")).await.unwrap();
    let request = server.take().await;
    assert_eq!(request.route(), ("GET", "/api/v1/inbox/conversations"));
    assert_eq!(request.query, query(&[("as", "bot_1")]));
    assert_eq!(conversations[0].members.len(), 2);
    assert_eq!(conversations[0].unread_count, 2);
    assert_eq!(
        conversations[0]
            .last_entry
            .as_ref()
            .and_then(InboxEntry::text),
        Some("last")
    );

    client.get_conversation("conv/1", None).await.unwrap();
    let request = server.take().await;
    assert_eq!(
        request.route(),
        ("GET", "/api/v1/inbox/conversations/conv%2F1")
    );
    assert!(request.query.is_empty());

    client
        .create_conversation(&CreateConversation::direct("bot_1"))
        .await
        .unwrap();
    let request = server.take().await;
    assert_eq!(request.route(), ("POST", "/api/v1/inbox/conversations"));
    assert_eq!(
        request.body,
        Some(json!({ "kind": "direct", "members": ["bot_1"] }))
    );

    client
        .create_conversation(&CreateConversation {
            encryption: Some(ConversationEncryption::E2e),
            ..CreateConversation::group("Launch", ["bot_1", "user_2"])
        })
        .await
        .unwrap();
    assert_eq!(
        server.take().await.body,
        Some(json!({
            "kind": "group",
            "title": "Launch",
            "members": ["bot_1", "user_2"],
            "encryption": "e2e",
        }))
    );

    client.rename_conversation("conv", None).await.unwrap();
    let request = server.take().await;
    assert_eq!(
        request.route(),
        ("PATCH", "/api/v1/inbox/conversations/conv")
    );
    assert_eq!(request.body, Some(json!({ "title": null })));

    client
        .add_conversation_members("conv", ["bot_2"])
        .await
        .unwrap();
    let request = server.take().await;
    assert_eq!(
        request.route(),
        ("POST", "/api/v1/inbox/conversations/conv/members")
    );
    assert_eq!(request.body, Some(json!({ "members": ["bot_2"] })));

    client
        .remove_conversation_member("conv", "bot 2")
        .await
        .unwrap();
    assert_eq!(
        server.take().await.route(),
        ("DELETE", "/api/v1/inbox/conversations/conv/members/bot%202")
    );
}

#[tokio::test]
async fn entry_pages_pass_the_window_and_expose_next_before() {
    let mut typing = entry("t");
    typing["kind"] = json!("typing");
    let mut server = Server::start(vec![ok(json!({
        "entries": [entry("b"), typing],
        "next_before": "1:a",
    }))])
    .await;
    let page = server
        .client()
        .list_inbox_entries(
            "conv",
            &InboxEntriesQuery {
                as_bot: Some("bot_1".into()),
                before: Some("9:z".into()),
                after: Some("1:0".into()),
                limit: Some(2),
            },
        )
        .await
        .unwrap();
    let request = server.take().await;
    assert_eq!(
        request.route(),
        ("GET", "/api/v1/inbox/conversations/conv/entries")
    );
    assert_eq!(
        request.query,
        query(&[
            ("as", "bot_1"),
            ("before", "9:z"),
            ("after", "1:0"),
            ("limit", "2")
        ])
    );
    assert_eq!(page.next_before.as_deref(), Some("1:a"));
    assert_eq!(page.entries[0].kind, InboxEntryKind::Message);
    assert_eq!(page.entries[1].kind, InboxEntryKind::Other("typing".into()));
}

#[tokio::test]
async fn history_streams_follow_next_before_to_the_end() {
    let mut server = Server::start(vec![
        ok(json!({ "entries": [entry("e"), entry("d")], "next_before": "2:d" })),
        ok(json!({ "entries": [entry("c"), entry("b")], "next_before": "1:b" })),
        ok(json!({ "entries": [entry("a")], "next_before": null })),
    ])
    .await;
    let start = InboxEntriesQuery {
        as_bot: Some("bot_1".into()),
        after: Some("0:start".into()),
        limit: Some(2),
        ..InboxEntriesQuery::default()
    };
    let ids: Vec<String> = server
        .client()
        .stream_inbox_entries("conv", start)
        .map_ok(|entry| entry.id)
        .try_collect()
        .await
        .unwrap();
    assert_eq!(ids, ["e", "d", "c", "b", "a"]);
    let mut queries = Vec::new();
    for _ in 0..3 {
        queries.push(server.take().await.query);
    }
    assert_eq!(
        queries,
        [
            query(&[("as", "bot_1"), ("after", "0:start"), ("limit", "2")]),
            query(&[
                ("as", "bot_1"),
                ("before", "2:d"),
                ("after", "0:start"),
                ("limit", "2")
            ]),
            query(&[
                ("as", "bot_1"),
                ("before", "1:b"),
                ("after", "0:start"),
                ("limit", "2")
            ]),
        ]
    );
}

#[tokio::test]
async fn history_streams_fetch_lazily_and_stop_on_errors() {
    let mut server = Server::start(vec![
        ok(json!({ "entries": [entry("b")], "next_before": "1:b" })),
        failure(404, "NOT_FOUND", "Conversation 'conv' not found"),
    ])
    .await;
    let mut history = server
        .client()
        .stream_inbox_entries("conv", InboxEntriesQuery::default());
    assert_eq!(history.next().await.unwrap().unwrap().id, "b");
    assert!(server.take().await.query.is_empty());
    assert!(history.next().await.unwrap().unwrap_err().is_not_found());
    assert!(history.next().await.is_none());
}

#[tokio::test]
async fn sends_wrap_plain_envelopes_and_generate_message_ids() {
    let mut server = Server::start(vec![ok(entry("sent")); 7]).await;
    let client = server.client();

    let sent = client.send_inbox_message("conv", "Hello").await.unwrap();
    assert_eq!(sent.text(), Some("sent"));
    let request = server.take().await;
    assert_eq!(
        request.route(),
        ("POST", "/api/v1/inbox/conversations/conv/entries")
    );
    let body = request.body.unwrap();
    assert!(is_message_id(&body["message_id"]), "{body}");
    assert_eq!(body["kind"], "message");
    assert_eq!(body["envelope"], plain_envelope(json!({ "text": "Hello" })));

    client
        .send_inbox_entry(
            "conv",
            &SendInboxEntry::reply("Retry", "m_a").with_message_id("msg_fixed"),
        )
        .await
        .unwrap();
    assert_eq!(
        server.take().await.body,
        Some(json!({
            "message_id": "msg_fixed",
            "kind": "message",
            "envelope": { "v": 1, "enc": "none", "body": { "text": "Retry", "reply_to": "m_a" } },
        }))
    );

    client
        .react_to_inbox_message("conv", "m_a", "👍", ReactionOp::Add)
        .await
        .unwrap();
    client
        .send_inbox_entry(
            "conv",
            &SendInboxEntry::reaction("m_a", "👍", ReactionOp::Remove)
                .with_message_id("msg_unreact"),
        )
        .await
        .unwrap();
    client
        .edit_inbox_message("conv", "m_a", "Fixed")
        .await
        .unwrap();
    client.delete_inbox_message("conv", "m_a").await.unwrap();
    let mut bodies = Vec::new();
    for _ in 0..4 {
        bodies.push(server.take().await.body.unwrap());
    }
    let sent: Vec<(Value, Value)> = bodies
        .iter()
        .map(|body| (body["kind"].clone(), body["envelope"]["body"].clone()))
        .collect();
    assert_eq!(
        sent,
        [
            (
                json!("reaction"),
                json!({ "target": "m_a", "emoji": "👍", "op": "add" })
            ),
            (
                json!("reaction"),
                json!({ "target": "m_a", "emoji": "👍", "op": "remove" })
            ),
            (json!("edit"), json!({ "target": "m_a", "text": "Fixed" })),
            (json!("delete"), json!({ "target": "m_a" })),
        ]
    );
    assert_eq!(bodies[1]["message_id"], "msg_unreact");
    for body in [&bodies[0], &bodies[2], &bodies[3]] {
        assert!(is_message_id(&body["message_id"]), "{body}");
    }

    let sealed = json!({ "v": 1, "enc": "hpke", "key_id": "k", "enc_key": "e", "ct": "c" });
    let envelopes = BTreeMap::from([
        ("bot_1".to_owned(), sealed.clone()),
        ("user_1".to_owned(), sealed.clone()),
    ]);
    client
        .send_inbox_entry(
            "conv",
            &SendInboxEntry::sealed(InboxEntryKind::Message, envelopes),
        )
        .await
        .unwrap();
    let body = server.take().await.body.unwrap();
    assert_eq!(
        body["envelopes"],
        json!({ "bot_1": sealed, "user_1": sealed })
    );
    assert!(body.get("envelope").is_none());
    assert!(is_message_id(&body["message_id"]), "{body}");
}

#[tokio::test]
async fn mark_read_always_sends_a_json_body() {
    let mut server = Server::start(vec![ok(json!({ "last_read_at": NOW })); 2]).await;
    let client = server.client();

    let marker = client.mark_conversation_read("conv", None).await.unwrap();
    assert_eq!(marker.last_read_at, NOW);
    let request = server.take().await;
    assert_eq!(
        request.route(),
        ("POST", "/api/v1/inbox/conversations/conv/read")
    );
    assert_eq!(request.headers["content-type"], "application/json");
    assert_eq!(request.body, Some(json!({})));

    client
        .mark_conversation_read("conv", Some("5:e"))
        .await
        .unwrap();
    assert_eq!(server.take().await.body, Some(json!({ "up_to": "5:e" })));
}

#[tokio::test]
async fn keys_are_fetched_registered_and_revoked() {
    let key = json!({
        "principal_id": "bot_1",
        "key_id": "k:1",
        "algorithm": "hpke-x25519-sha256-chacha20poly1305",
        "public_key": "cHVi",
        "created_at": NOW,
    });
    let mut server = Server::start(vec![
        ok(json!({ "keys": [key.clone()] })),
        ok(key.clone()),
        no_content(),
        no_content(),
    ])
    .await;
    let client = server.client();

    let keys = client.get_inbox_keys(["user_1", "bot_1"]).await.unwrap();
    assert_eq!(keys[0].key_id, "k:1");
    let request = server.take().await;
    assert_eq!(request.route(), ("GET", "/api/v1/inbox/keys"));
    assert_eq!(request.query, query(&[("principals", "user_1,bot_1")]));

    let registered = client
        .register_inbox_key(
            &RegisterInboxKey::new("k:1", "hpke-x25519-sha256-chacha20poly1305", "cHVi")
                .for_bot("bot_1"),
        )
        .await
        .unwrap();
    assert_eq!(registered.public_key, "cHVi");
    let request = server.take().await;
    assert_eq!(request.route(), ("PUT", "/api/v1/inbox/keys"));
    assert_eq!(
        request.body,
        Some(json!({
            "principal_id": "bot_1",
            "key_id": "k:1",
            "algorithm": "hpke-x25519-sha256-chacha20poly1305",
            "public_key": "cHVi",
        }))
    );

    client.revoke_inbox_key("k:1", Some("bot_1")).await.unwrap();
    let request = server.take().await;
    assert_eq!(request.route(), ("DELETE", "/api/v1/inbox/keys/k:1"));
    assert_eq!(request.query, query(&[("principal_id", "bot_1")]));
    client.revoke_inbox_key("k:2", None).await.unwrap();
    assert!(server.take().await.query.is_empty());
}

#[test]
fn message_ids_are_unique_and_valid() {
    let ids: BTreeSet<String> = (0..1000).map(|_| new_inbox_message_id()).collect();
    assert_eq!(ids.len(), 1000);
    assert!(ids.iter().all(|id| is_message_id(&json!(id))));
}

#[test]
fn entries_expose_plain_bodies_and_text_but_not_sealed_ones() {
    let parse = |value: Value| serde_json::from_value::<InboxEntry>(value).unwrap();
    let mut reaction = entry("r");
    reaction["kind"] = json!("reaction");
    reaction["envelope"] = plain_envelope(json!({ "target": "m_a", "emoji": "👍", "op": "add" }));
    let mut sealed = entry("s");
    sealed["envelope"] = json!({
        "v": 1, "enc": "hpke", "key_id": "k", "enc_key": "e", "ct": "c",
        "body": { "text": "not plain" },
    });

    assert_eq!(parse(entry("hello")).text(), Some("hello"));
    let reaction = parse(reaction);
    assert_eq!(reaction.text(), None);
    assert_eq!(reaction.body().unwrap()["emoji"], "👍");
    let sealed = parse(sealed);
    assert_eq!(sealed.body(), None);
    assert_eq!(sealed.text(), None);
    assert_eq!(
        serde_json::to_value(InboxEntryKind::Other("typing".into())).unwrap(),
        json!("typing")
    );
}
