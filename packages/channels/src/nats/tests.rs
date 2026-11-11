use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_types::channel::{ChannelPushKind, now_unix};
use serde_json::json;

fn push(kind: ChannelPushKind, request: Option<&str>, value: serde_json::Value) -> ChannelPush {
    ChannelPush {
        channel_id: "run".into(),
        request_id: request.map(str::to_owned),
        kind,
        value,
    }
}

#[test]
fn credentials_are_scoped_and_signed() {
    let account = nkeys::KeyPair::new_account();
    let seed = account.seed().unwrap();
    for role in [Role::Client, Role::Executor] {
        let jwt = mint_user_token(&seed, "run", role, 60).unwrap();
        let parts: Vec<_> = jwt.split('.').collect();
        account
            .verify(
                format!("{}.{}", parts[0], parts[1]).as_bytes(),
                &URL_SAFE_NO_PAD.decode(parts[2]).unwrap(),
            )
            .unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        let (publish, subscribe) = if role == Role::Client {
            ("channel.run.inbox", "channel.run.inbox.ack.>")
        } else {
            ("channel.run.inbox.ack.>", "channel.run.inbox")
        };
        assert_eq!(claims["nats"]["pub"]["allow"], json!([publish]));
        assert_eq!(claims["nats"]["sub"]["allow"], json!([subscribe]));
        assert_eq!(claims["nats"]["bearer_token"], true);
        assert_eq!(
            claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
            60
        );
    }
    assert!(mint_user_token(&seed, "run.*", Role::Client, 60).is_err());
    assert!(mint_user_token(&seed, "run", Role::Client, 0).is_err());
    assert!(
        mint_user_token(
            &nkeys::KeyPair::new_user().seed().unwrap(),
            "run",
            Role::Client,
            60
        )
        .is_err()
    );
    assert!(validate_url("nats://user:secret@host:4222").is_err());
}

#[test]
fn credential_refresh_preserves_absolute_deadline() {
    let seed = nkeys::KeyPair::new_account().seed().unwrap();
    let deadline = now_unix() + 120;
    let token = mint_user_token_until(&seed, "run", Role::Client, deadline).unwrap();
    let payload = token.split('.').nth(1).unwrap();
    let claims: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
    assert_eq!(claims["exp"], deadline);
    assert!(mint_user_token_until(&seed, "run", Role::Client, now_unix()).is_err());
    assert!(mint_user_token_until(&seed, "run", Role::Client, now_unix() + 10 * 60 * 60).is_err());
}

#[tokio::test]
async fn receiver_confirms_once_and_rejects_foreign_or_conflicting_pushes() {
    let router = router::Router::new("run");
    router.register("request", now_unix() + 60).unwrap();
    let receiver = router.take_receiver("request").unwrap();
    let reply = push(
        ChannelPushKind::Reply,
        Some("request"),
        json!({"ok":true,"nested":{"a":1,"b":2}}),
    );
    assert!(
        router
            .route(Envelope {
                id: "delivery".into(),
                push: reply.clone()
            })
            .accepted
    );
    assert_eq!(
        receiver.await.unwrap(),
        json!({"ok":true,"nested":{"a":1,"b":2}})
    );
    router.remove("request");
    let mut reordered = reply.clone();
    reordered.value = serde_json::from_str(r#"{"nested":{"b":2,"a":1},"ok":true}"#).unwrap();
    assert!(
        router
            .route(Envelope {
                id: "delivery".into(),
                push: reordered
            })
            .accepted
    );
    assert!(
        router
            .route(Envelope {
                id: "delivery".into(),
                push: reply.clone()
            })
            .accepted
    );
    assert!(
        !router
            .route(Envelope {
                id: "different".into(),
                push: reply.clone()
            })
            .accepted
    );
    let mut conflict = reply.clone();
    conflict.value = json!(false);
    assert!(
        !router
            .route(Envelope {
                id: "delivery".into(),
                push: conflict
            })
            .accepted
    );
    let mut foreign = reply;
    foreign.channel_id = "other".into();
    assert!(
        !router
            .route(Envelope {
                id: "foreign".into(),
                push: foreign
            })
            .accepted
    );
    let inbound = push(ChannelPushKind::Inbound, None, json!("steer"));
    assert!(
        router
            .route(Envelope {
                id: "steer".into(),
                push: inbound.clone()
            })
            .accepted
    );
    assert!(
        router
            .route(Envelope {
                id: "steer".into(),
                push: inbound
            })
            .accepted
    );
    assert_eq!(router.drain(), vec![json!("steer")]);
    assert!(
        router
            .route(Envelope {
                id: "stop".into(),
                push: push(ChannelPushKind::Cancel, None, json!(null))
            })
            .accepted
    );
    assert!(router.cancelled.is_cancelled());
    assert!(
        !router
            .route(Envelope {
                id: "late".into(),
                push: push(ChannelPushKind::Inbound, None, json!("late"))
            })
            .accepted
    );
}

#[tokio::test]
async fn receiver_bounds_queue_and_rejects_expired_requests() {
    let router = router::Router::new("run");
    router.register("expired", now_unix() - 1).unwrap();
    assert!(
        !router
            .route(Envelope {
                id: "expired".into(),
                push: push(ChannelPushKind::Reply, Some("expired"), json!(1))
            })
            .accepted
    );
    for i in 0..8 {
        assert!(
            router
                .route(Envelope {
                    id: format!("push-{i}"),
                    push: push(ChannelPushKind::Inbound, None, json!(i))
                })
                .accepted
        );
    }
    assert!(
        !router
            .route(Envelope {
                id: "full".into(),
                push: push(ChannelPushKind::Inbound, None, json!(9))
            })
            .accepted
    );
    assert_eq!(router.drain().len(), 8);
    router.close();
    assert!(router.register("closed", now_unix() + 60).is_err());
}

/// The kind the API reads from the waiter's acknowledgement, or `None` when it was accepted.
fn refusal(router: &router::Router, id: &str, push: ChannelPush) -> Option<ChannelRejection> {
    let ack = router.route(Envelope {
        id: id.into(),
        push,
    });
    (!ack.accepted).then(|| match ack.rejection() {
        ChannelForwardError::Rejected { kind, .. } => kind,
        other => panic!("{other:?}"),
    })
}

#[test]
fn request_refusals_reach_the_api_as_typed_rejections() {
    let router = router::Router::new("run");
    let reply = |request: &str, value| push(ChannelPushKind::Reply, Some(request), value);
    router.register("answered", now_unix() + 60).unwrap();
    router.register("expired", now_unix() - 1).unwrap();
    router.register("abandoned", now_unix() + 60).unwrap();
    drop(router.take_receiver("abandoned"));

    let answer = reply("answered", json!(1));
    assert_eq!(refusal(&router, "first", answer), None);
    let mut foreign = reply("answered", json!(1));
    foreign.channel_id = "other".into();
    let refused = [
        (
            "second",
            reply("answered", json!(2)),
            ChannelRejection::AlreadyAnswered,
        ),
        (
            "first",
            reply("answered", json!(3)),
            ChannelRejection::DuplicateDelivery,
        ),
        (
            "late",
            reply("expired", json!(1)),
            ChannelRejection::Expired,
        ),
        (
            "unknown",
            reply("missing", json!(1)),
            ChannelRejection::UnknownRequest,
        ),
        (
            "gone",
            reply("abandoned", json!(1)),
            ChannelRejection::Closed,
        ),
        ("foreign", foreign, ChannelRejection::WrongChannel),
    ];
    for (id, push, kind) in refused {
        assert_eq!(refusal(&router, id, push), Some(kind), "{id}");
    }
}

#[test]
fn channel_refusals_reach_the_api_as_typed_rejections() {
    let router = router::Router::new("run");
    let steer = |value: i32| push(ChannelPushKind::Inbound, None, json!(value));
    let cancel = || push(ChannelPushKind::Cancel, None, json!(null));
    for i in 0..8 {
        assert_eq!(refusal(&router, &format!("steer-{i}"), steer(i)), None);
    }
    let full = refusal(&router, "full", steer(8));
    assert_eq!(full, Some(ChannelRejection::InboundFull));
    assert_eq!(refusal(&router, "stop", cancel()), None);
    let cancelled = refusal(&router, "after-stop", steer(9));
    assert_eq!(cancelled, Some(ChannelRejection::Cancelled));
    router.close();
    let closed = refusal(&router, "closed", cancel());
    assert_eq!(closed, Some(ChannelRejection::Closed));

    for error in [Some("A reason from a newer waiter".to_owned()), None] {
        let ack = Acknowledgement {
            accepted: false,
            error,
        };
        assert!(matches!(
            ack.rejection(),
            ChannelForwardError::Rejected { kind: ChannelRejection::Other, reason } if !reason.is_empty()
        ));
    }
}

#[tokio::test]
async fn realtime_publisher_bounds_a_hung_broker_then_fails_fast() {
    let hung = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let forwarder = std::sync::Arc::new(
        NatsForwarder::new(
            format!("nats://{}", hung.local_addr().unwrap()),
            nkeys::KeyPair::new_account().seed().unwrap(),
        )
        .unwrap(),
    );
    let scope = "a".repeat(64);
    let payload = br#"{"protocol_version":1,"kind":"invalidate","nonce":"n"}"#;
    let started = std::time::Instant::now();
    let publishes: Vec<_> = (0..8)
        .map(|_| {
            let forwarder = forwarder.clone();
            let scope = scope.clone();
            tokio::spawn(async move { forwarder.publish_realtime(&scope, payload).await })
        })
        .collect();
    for publish in publishes {
        assert!(publish.await.unwrap().is_err());
    }
    assert!(
        started.elapsed() < Duration::from_millis(1800),
        "concurrent wakeups must share one bounded connect: {:?}",
        started.elapsed()
    );
    let started = std::time::Instant::now();
    let error = forwarder
        .publish_realtime(&scope, payload)
        .await
        .unwrap_err();
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "{:?}",
        started.elapsed()
    );
    assert!(error.to_string().contains("unavailable"), "{error}");
}

type Frames = tokio::sync::mpsc::UnboundedSender<Vec<u8>>;

/// Speaks just enough NATS for these tests: the handshake, PING, SUB, skipped publications and
/// frames a test injects. While down it drops live connections and refuses new ones.
struct FakeBroker {
    url: String,
    up: tokio::sync::watch::Sender<bool>,
    refused: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    subscriptions: tokio::sync::mpsc::UnboundedReceiver<(String, Frames)>,
}

impl FakeBroker {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("nats://{}", listener.local_addr().unwrap());
        let (up, live) = tokio::sync::watch::channel(true);
        let refused = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (subscribed, subscriptions) = tokio::sync::mpsc::unbounded_channel();
        let counter = refused.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                if *live.borrow() {
                    tokio::spawn(fake_session(stream, live.clone(), subscribed.clone()));
                } else {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
        });
        Self {
            url,
            up,
            refused,
            subscriptions,
        }
    }

    /// The next `SUB`: its sid, and a way to send frames to the subscribing connection.
    async fn subscribed(&mut self) -> (String, Frames) {
        tokio::time::timeout(Duration::from_secs(5), self.subscriptions.recv())
            .await
            .expect("no subscription reached the broker")
            .unwrap()
    }

    async fn refusals(&self, at_least: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.refused.load(std::sync::atomic::Ordering::SeqCst) < at_least {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("too few connection attempts reached the broker while it was down");
    }

    fn grant(&self, channel_id: &str) -> flow_like_types::channel::ChannelGrant {
        use flow_like_types::channel::{
            ChannelClientDescriptor, ChannelExecutorGrant, ChannelGrant, ChannelHandle,
        };
        let expires_at = now_unix() + 60;
        ChannelGrant {
            channel_id: channel_id.into(),
            expires_at,
            executor: ChannelExecutorGrant::Nats {
                url: self.url.clone(),
                subject: subject_for(channel_id).unwrap(),
                token: "executor-jwt".into(),
            },
            client: ChannelHandle {
                channel_id: channel_id.into(),
                request_id: None,
                expires_at,
                transport: ChannelClientDescriptor::Http {
                    push_url: format!("https://api.test/api/v1/channels/{channel_id}/push"),
                    token: "responder".into(),
                },
                fallback: None,
            },
        }
    }
}

async fn fake_session(
    stream: tokio::net::TcpStream,
    mut live: tokio::sync::watch::Receiver<bool>,
    subscribed: tokio::sync::mpsc::UnboundedSender<(String, Frames)>,
) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
    let (reader, mut writer) = stream.into_split();
    let (frames, mut queued) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let info = r#"INFO {"server_id":"fake","proto":1,"headers":true,"max_payload":1048576}"#;
    let _ = frames.send(format!("{info}\r\n").into_bytes());
    let session = async {
        let mut reader = tokio::io::BufReader::new(reader);
        let mut line = String::new();
        while reader.read_line(&mut line).await.is_ok_and(|read| read > 0) {
            match line.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["PING"] => {
                    let _ = frames.send(b"PONG\r\n".to_vec());
                }
                ["SUB", _, sid] => {
                    let _ = subscribed.send((sid.to_string(), frames.clone()));
                }
                ["PUB" | "HPUB", .., size] => {
                    let mut payload = vec![0; size.parse::<usize>().unwrap_or_default() + 2];
                    if reader.read_exact(&mut payload).await.is_err() {
                        return;
                    }
                }
                _ => {}
            }
            line.clear();
        }
    };
    let writing = async {
        while let Some(frame) = queued.recv().await {
            if writer.write_all(&frame).await.is_err() {
                return;
            }
        }
    };
    tokio::select! {
        _ = live.wait_for(|up| !*up) => {}
        _ = session => {}
        _ = writing => {}
    }
}

fn message_frame(subject: &str, sid: &str, reply: Option<&str>, payload: &[u8]) -> Vec<u8> {
    let reply = reply.map(|reply| format!("{reply} ")).unwrap_or_default();
    let mut frame = format!("MSG {subject} {sid} {reply}{}\r\n", payload.len()).into_bytes();
    frame.extend_from_slice(payload);
    frame.extend_from_slice(b"\r\n");
    frame
}

/// Delivers `push` to a channel's inbox the way the API's forwarder does.
fn delivery(channel_id: &str, sid: &str, id: &str, push: &ChannelPush) -> Vec<u8> {
    let subject = subject_for(channel_id).unwrap();
    let envelope = serde_json::to_vec(&Envelope {
        id: id.into(),
        push: push.clone(),
    })
    .unwrap();
    let acks = format!("{subject}.ack.{id}");
    message_frame(&subject, sid, Some(&acks), &envelope)
}

#[tokio::test]
async fn pending_requests_outlive_a_client_that_stopped_reconnecting() {
    use flow_like_types::channel::ChannelOutcome;
    let mut broker = FakeBroker::start().await;
    let grant = broker.grant("run");
    let channel = NatsChannel::connect_inbox(&grant, 1, Duration::from_millis(20))
        .await
        .unwrap();
    broker.subscribed().await;
    let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
    let waiter = tokio::spawn({
        let (channel, ticket) = (channel.clone(), ticket.clone());
        async move { channel.wait(&ticket, None).await.unwrap() }
    });

    broker.up.send_replace(false);
    // The client's own budget is one attempt; every further refusal is the inbox retrying.
    broker.refusals(3).await;
    assert!(
        !waiter.is_finished(),
        "a broker outage closed a pending request"
    );

    broker.up.send_replace(true);
    let (sid, frames) = broker.subscribed().await;
    let reply = push(
        ChannelPushKind::Reply,
        Some(ticket.request_id.as_str()),
        json!(42),
    );
    frames
        .send(delivery("run", &sid, "after-outage", &reply))
        .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("the reply never reached the waiter")
        .unwrap();
    assert_eq!(outcome, ChannelOutcome::Responded(json!(42)));
    channel.close().await;
}

#[tokio::test]
async fn confirmed_publications_report_missing_listeners_and_refusals_as_types() {
    let mut broker = FakeBroker::start().await;
    let client = connect(&broker.url, "client-jwt").await.unwrap();
    let subject = subject_for("run").unwrap();
    let cases = [
        ("unheard", None, ChannelForwardError::NoListener),
        (
            "refused",
            Some(r#"{"accepted":false,"error":"Request has expired"}"#),
            ChannelForwardError::Rejected {
                kind: ChannelRejection::Expired,
                reason: "Request has expired".into(),
            },
        ),
    ];
    for (id, answer, expected) in cases {
        let publication = tokio::spawn({
            let (client, subject) = (client.clone(), subject.clone());
            let reply = push(ChannelPushKind::Reply, Some("request"), json!(1));
            async move { publish_confirmed(&client, &subject, &reply, id).await }
        });
        let (sid, frames) = broker.subscribed().await;
        let acks = format!("{subject}.ack.{id}");
        let frame = match answer {
            None => format!("HMSG {acks} {sid} 16 16\r\nNATS/1.0 503\r\n\r\n\r\n").into_bytes(),
            Some(ack) => message_frame(&acks, &sid, None, ack.as_bytes()),
        };
        frames.send(frame).unwrap();
        let error = publication.await.unwrap().unwrap_err();
        assert_eq!(
            error.downcast_ref::<ChannelForwardError>(),
            Some(&expected),
            "{error:#}"
        );
    }
}

struct Broker {
    process: std::process::Child,
    directory: std::path::PathBuf,
    url: String,
    websocket_url: String,
    seed: String,
}

impl Broker {
    async fn start() -> Self {
        let binary = std::env::var("NATS_SERVER_BIN")
            .expect("set NATS_SERVER_BIN to the nats-server executable");
        let operator = nkeys::KeyPair::new_operator();
        let account = nkeys::KeyPair::new_account();
        let now = now_unix();
        let operator_jwt = token::sign(&operator, json!({"iss":operator.public_key(),"sub":operator.public_key(),"iat":now,"nats":{"type":"operator","version":2}})).unwrap();
        let account_jwt = token::sign(&operator, json!({"iss":operator.public_key(),"sub":account.public_key(),"iat":now,"nats":{"type":"account","version":2,"limits":{"subs":-1,"data":-1,"payload":1048576,"imports":-1,"exports":-1,"wildcards":true,"conn":-1,"leaf":-1},"disallow_bearer":false}})).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let websocket_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let directory = std::env::temp_dir().join(format!(
            "flow-like-nats-test-{}",
            flow_like_types::channel::new_request_id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let config = directory.join("nats.conf");
        std::fs::write(&config, format!("listen: 127.0.0.1:{port}\nwebsocket {{ host: 127.0.0.1, port: {websocket_port}, no_tls: true }}\noperator: {operator_jwt}\nresolver: MEMORY\nresolver_preload: {{ {}: {account_jwt} }}\n",account.public_key())).unwrap();
        let log = std::fs::File::create(directory.join("broker.log")).unwrap();
        let process = std::process::Command::new(binary)
            .arg("-c")
            .arg(&config)
            .stdout(std::process::Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        let mut broker = Self {
            process,
            directory,
            url: format!("nats://127.0.0.1:{port}"),
            websocket_url: format!("ws://127.0.0.1:{websocket_port}"),
            seed: account.seed().unwrap(),
        };
        for _ in 0..100 {
            if broker.process.try_wait().unwrap().is_some() {
                panic!(
                    "broker failed: {}",
                    std::fs::read_to_string(broker.directory.join("broker.log")).unwrap()
                );
            }
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                return broker;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("NATS test broker failed to listen");
    }

    fn grant(&self, channel_id: &str) -> flow_like_types::channel::ChannelGrant {
        use flow_like_types::channel::*;
        let subject = subject_for(channel_id).unwrap();
        let expires_at = now_unix() + 60;
        ChannelGrant {
            channel_id: channel_id.into(),
            expires_at,
            executor: ChannelExecutorGrant::Nats {
                url: self.url.clone(),
                subject: subject.clone(),
                token: mint_user_token(&self.seed, channel_id, Role::Executor, 60).unwrap(),
            },
            client: ChannelHandle {
                channel_id: channel_id.into(),
                request_id: None,
                expires_at,
                transport: ChannelClientDescriptor::Nats {
                    url: self.url.clone(),
                    subject,
                    token: mint_user_token(&self.seed, channel_id, Role::Client, 60).unwrap(),
                    expires_at,
                },
                fallback: None,
            },
        }
    }

    async fn restart(&mut self) {
        self.process.kill().unwrap();
        self.process.wait().unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let log = std::fs::OpenOptions::new()
            .append(true)
            .open(self.directory.join("broker.log"))
            .unwrap();
        self.process = std::process::Command::new(std::env::var("NATS_SERVER_BIN").unwrap())
            .arg("-c")
            .arg(self.directory.join("nats.conf"))
            .stdout(std::process::Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(self.url.trim_start_matches("nats://"))
                .await
                .is_ok()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("NATS broker did not restart");
    }
}

#[tokio::test]
#[ignore = "requires NATS_SERVER_BIN and permission to open local sockets"]
async fn real_broker_realtime_receiver_is_read_only_and_account_scoped() {
    use flow_like_types::realtime::RealtimeTransport;
    use futures_util::StreamExt;
    use std::sync::Arc;
    use tokio::sync::{Notify, watch};

    let mut broker = Broker::start().await;
    let scope = "a".repeat(64);
    let other_scope = "b".repeat(64);
    let subject = realtime_subject(&scope).unwrap();
    let other_subject = realtime_subject(&other_scope).unwrap();
    let token = mint_realtime_subscriber_token(&broker.seed, &scope, 120).unwrap();
    let transport = RealtimeTransport::Nats {
        url: broker.websocket_url.clone(),
        subject: subject.clone(),
        token: token.clone(),
    };
    let wake = Arc::new(Notify::new());
    let (healthy, mut ready) = watch::channel(false);
    let notifications = Arc::clone(&wake);
    let listener = tokio::spawn(async move {
        crate::realtime::listen(&transport, notifications, healthy, None).await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !*ready.borrow() {
            ready.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    wake.notified().await;

    let publisher = connect(
        &broker.url,
        &mint_realtime_publisher_token(&broker.seed, 120).unwrap(),
    )
    .await
    .unwrap();
    let payload =
        serde_json::to_vec(&json!({"protocol_version":1,"kind":"invalidate","nonce":"trusted"}))
            .unwrap();
    publisher
        .publish(other_subject.clone(), payload.clone().into())
        .await
        .unwrap();
    publisher
        .publish(
            subject.clone(),
            b"untrusted execution payload".to_vec().into(),
        )
        .await
        .unwrap();
    publisher.flush().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), wake.notified())
            .await
            .is_err()
    );
    publisher
        .publish(subject.clone(), payload.clone().into())
        .await
        .unwrap();
    publisher.flush().await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), wake.notified())
        .await
        .unwrap();

    // The same receiver credential cannot inject even a correctly shaped wakeup.
    let receiver = connect(&broker.url, &token).await.unwrap();
    receiver
        .publish(subject.clone(), payload.clone().into())
        .await
        .unwrap();
    receiver.flush().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), wake.notified())
            .await
            .is_err()
    );

    // A receiver cannot widen its subscription to another account.
    let mut foreign = receiver.subscribe(other_subject.clone()).await.unwrap();
    receiver.flush().await.unwrap();
    publisher
        .publish(other_subject, payload.clone().into())
        .await
        .unwrap();
    publisher.flush().await.unwrap();
    assert!(!matches!(
        tokio::time::timeout(Duration::from_millis(200), foreign.next()).await,
        Ok(Some(_))
    ));

    broker.restart().await;
    let stopped = tokio::time::timeout(Duration::from_secs(5), listener)
        .await
        .unwrap()
        .unwrap();
    assert!(
        stopped.is_err(),
        "provider loss must return control to HTTP replay"
    );
}

impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[tokio::test]
#[ignore = "requires NATS_SERVER_BIN and permission to open local sockets"]
async fn real_broker_reply_steer_cancel_forwarder_permissions_and_expiry() {
    use crate::ChannelForwarder;
    use flow_like_types::channel::{ChannelClientDescriptor, ChannelOutcome};
    let mut broker = Broker::start().await;
    let grant = broker.grant("run");
    let channel = NatsChannel::connect(&grant).await.unwrap();
    let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
    let ChannelClientDescriptor::Nats { token, subject, .. } = &grant.client.transport else {
        panic!()
    };
    let client = connect(&broker.url, token).await.unwrap();
    let response = push(
        ChannelPushKind::Reply,
        Some(&ticket.request_id),
        json!({"answer":42}),
    );
    publish_confirmed(&client, subject, &response, "reply-id")
        .await
        .unwrap();
    assert_eq!(
        channel.wait(&ticket, None).await.unwrap(),
        ChannelOutcome::Responded(json!({"answer":42}))
    );

    // A lost acknowledgement followed by an HTTP retry keeps one accepted delivery.
    let inbound = push(ChannelPushKind::Inbound, None, json!("steer"));
    publish_confirmed(&client, subject, &inbound, "steer-id")
        .await
        .unwrap();
    let forwarder = NatsForwarder::new(broker.url.clone(), broker.seed.clone()).unwrap();
    forwarder
        .forward_with_id(&inbound, "steer-id")
        .await
        .unwrap();
    assert_eq!(channel.drain_inbound().await, vec![json!("steer")]);

    // A run's publisher cannot inject a message into another run's receiver.
    let other_grant = broker.grant("other");
    let other = NatsChannel::connect(&other_grant).await.unwrap();
    let mut foreign = inbound.clone();
    foreign.channel_id = "other".into();
    let payload = serde_json::to_vec(&Envelope {
        id: "foreign-id".into(),
        push: foreign,
    })
    .unwrap();
    client
        .publish_with_reply(
            "channel.other.inbox",
            "channel.other.inbox.ack.foreign-id",
            payload.into(),
        )
        .await
        .unwrap();
    client.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(other.drain_inbound().await.is_empty());

    let expired = mint_user_token(&broker.seed, "run", Role::Client, 1).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(connect(&broker.url, &expired).await.is_err());

    broker.restart().await;
    let after_restart = channel.open(Duration::from_secs(10)).await.unwrap();
    let reply = push(
        ChannelPushKind::Reply,
        Some(&after_restart.request_id),
        json!("reconnected"),
    );
    publish_confirmed(&client, subject, &reply, "reconnected-id")
        .await
        .unwrap();
    assert_eq!(
        channel.wait(&after_restart, None).await.unwrap(),
        ChannelOutcome::Responded(json!("reconnected"))
    );

    let pending = channel.open(Duration::from_secs(10)).await.unwrap();
    forwarder
        .forward_with_id(
            &push(ChannelPushKind::Cancel, None, json!(null)),
            "cancel-id",
        )
        .await
        .unwrap();
    assert_eq!(
        channel.wait(&pending, None).await.unwrap(),
        ChannelOutcome::Cancelled
    );
    assert!(channel.is_cancelled().await);
    other.close().await;
    channel.close().await;
}

#[tokio::test]
#[ignore = "requires NATS_SERVER_BIN and permission to open local sockets"]
async fn real_broker_reconnect_presents_a_freshly_minted_credential() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut broker = Broker::start().await;
    let minted = std::sync::Arc::new(AtomicUsize::new(0));
    let client = connect_with(&broker.url, MAX_RECONNECTS, CONNECT_TIMEOUT, {
        let (minted, seed) = (minted.clone(), broker.seed.clone());
        move || {
            minted.fetch_add(1, Ordering::SeqCst);
            mint_realtime_publisher_token(&seed, 60)
        }
    })
    .await
    .unwrap();
    assert_eq!(minted.load(Ordering::SeqCst), 1);
    broker.restart().await;
    tokio::time::timeout(CONNECT_TIMEOUT, client.flush())
        .await
        .unwrap()
        .unwrap();
    assert!(minted.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
#[ignore = "requires NATS_SERVER_BIN and permission to open local sockets"]
async fn real_broker_realtime_publishing_recovers_after_a_restart() {
    use futures_util::StreamExt;

    let mut broker = Broker::start().await;
    let scope = "a".repeat(64);
    let token = mint_realtime_subscriber_token(&broker.seed, &scope, 120).unwrap();
    let payload = br#"{"protocol_version":1,"kind":"invalidate","nonce":"n"}"#;
    let forwarder = NatsForwarder::new(broker.url.clone(), broker.seed.clone()).unwrap();
    for round in 0..2 {
        if round == 1 {
            broker.restart().await;
        }
        let receiver = connect(&broker.url, &token).await.unwrap();
        let mut wakeups = receiver
            .subscribe(realtime_subject(&scope).unwrap())
            .await
            .unwrap();
        receiver.flush().await.unwrap();
        let delivered = tokio::time::timeout(Duration::from_secs(10), async {
            while forwarder.publish_realtime(&scope, payload).await.is_err()
                || !matches!(
                    tokio::time::timeout(Duration::from_millis(500), wakeups.next()).await,
                    Ok(Some(_))
                )
            {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await;
        assert!(delivered.is_ok(), "round {round}");
    }
}
