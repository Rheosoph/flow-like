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

struct Broker {
    process: std::process::Child,
    directory: std::path::PathBuf,
    url: String,
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
        let directory = std::env::temp_dir().join(format!(
            "flow-like-nats-test-{}",
            flow_like_types::channel::new_request_id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let config = directory.join("nats.conf");
        std::fs::write(&config, format!("listen: 127.0.0.1:{port}\noperator: {operator_jwt}\nresolver: MEMORY\nresolver_preload: {{ {}: {account_jwt} }}\n",account.public_key())).unwrap();
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
