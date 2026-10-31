use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn jwt(subject: &str, expiry: u64) -> String {
    format!(
        "e30.{}.signature",
        URL_SAFE_NO_PAD.encode(json!({"sub":subject,"exp":expiry}).to_string())
    )
}

fn user(subject: &str, expiry: u64) -> Value {
    json!({"access_token":jwt(subject,expiry),"refresh_token":"refresh-first","expires_at":expiry,"token_type":"Bearer","profile":{"sub":subject}})
}

async fn install(hub: &str, endpoint: &str, expiry: u64) -> Snapshot {
    let snapshot = native_auth_read("profile".into(), hub.into())
        .await
        .unwrap();
    write_session(
        "profile".into(),
        hub.into(),
        snapshot.generation,
        Configuration {
            authority: "https://issuer.test".into(),
            client_id: "client".into(),
            token_endpoint: endpoint.into(),
            ..Default::default()
        },
        user("alice", expiry).to_string(),
    )
    .await
    .unwrap()
}

async fn server(
    responses: Vec<(u16, Value)>,
) -> (
    String,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<Vec<String>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let task = tokio::spawn(async move {
        let mut bodies = Vec::new();
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0u8; 4096];
                let size = socket.read(&mut chunk).await.unwrap();
                assert!(size > 0);
                bytes.extend_from_slice(&chunk[..size]);
                if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..index]);
                    let size = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|size| size.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= index + 4 + size {
                        break;
                    }
                }
            }
            bodies.push(String::from_utf8(bytes).unwrap());
            count.fetch_add(1, Ordering::SeqCst);
            let body = body.to_string();
            socket.write_all(format!("HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        bodies
    });
    (endpoint, requests, task)
}

fn unique_hub() -> String {
    format!("https://{}.test", uuid::Uuid::new_v4())
}

async fn stored_user(hub: &str) -> Value {
    let (key, session) = session("profile", hub).await.unwrap();
    let mut session = session.lock().await;
    session.load(&key).await.unwrap().user.clone().unwrap()
}

#[tokio::test]
async fn concurrent_refresh_reuses_rotation_and_rejected_token_retries_once() {
    let expiry = now_seconds() + 3600;
    let next = jwt("alice", expiry);
    let (endpoint, count, server) = server(vec![
        (200, json!({"access_token":next,"refresh_token":"refresh-next","token_type":"Bearer","expires_in":3600})),
        (200, json!({"access_token":jwt("alice",expiry+1),"token_type":"Bearer","expires_in":3601})),
    ]).await;
    let hub = unique_hub();
    install(&hub, &endpoint, 1).await;
    let binding = SessionBinding::capture("profile", &hub)
        .await
        .unwrap()
        .unwrap();
    let mut jobs = Vec::new();
    for _ in 0..12 {
        let binding = binding.clone();
        jobs.push(tokio::spawn(async move {
            binding.access_token(None).await.unwrap()
        }));
    }
    for job in jobs {
        assert_eq!(job.await.unwrap(), next);
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_ne!(binding.access_token(Some(&next)).await.unwrap(), next);
    let requests = server.await.unwrap();
    assert!(requests[0].contains("refresh_token=refresh-first"));
    assert!(requests[1].contains("refresh_token=refresh-next"));
    let snapshot = native_auth_read("profile".into(), hub.clone())
        .await
        .unwrap();
    let saved: Value = serde_json::from_str(snapshot.user.as_deref().unwrap()).unwrap();
    assert!(saved.get("refresh_token").is_none());
    assert_eq!(stored_user(&hub).await["refresh_token"], "refresh-next");
}

#[tokio::test]
async fn logout_fences_same_account_relogin_and_stale_frontend_writes() {
    let hub = unique_hub();
    let first = install(&hub, "https://issuer.test/token", now_seconds() + 3600).await;
    let binding = SessionBinding::capture("profile", &hub)
        .await
        .unwrap()
        .unwrap();
    native_auth_clear("profile".into(), hub.clone(), first.generation.clone())
        .await
        .unwrap();
    assert!(!binding.is_current());
    install(&hub, "https://issuer.test/token", now_seconds() + 3600).await;
    assert!(binding.access_token(None).await.is_err());
    assert!(binding.cached_access_token().await.is_err());
    assert!(
        native_auth_clear("profile".into(), hub.clone(), first.generation.clone())
            .await
            .is_err()
    );
    assert!(
        write_session(
            "profile".into(),
            hub.clone(),
            first.generation,
            Configuration {
                authority: "https://issuer.test".into(),
                client_id: "client".into(),
                token_endpoint: "https://issuer.test/token".into(),
                ..Default::default()
            },
            user("alice", now_seconds() + 3600).to_string()
        )
        .await
        .is_err()
    );
    assert_eq!(
        current_subject("profile", &hub).await.unwrap().as_deref(),
        Some("alice")
    );
}

#[tokio::test]
async fn transient_provider_error_preserves_grant_but_invalid_grant_clears_it() {
    let (endpoint, _, server) = server(vec![
        (503, json!({"error":"temporarily_unavailable"})),
        (
            400,
            json!({"error":"invalid_grant","error_description":"must never reach diagnostics"}),
        ),
    ])
    .await;
    let hub = unique_hub();
    install(&hub, &endpoint, 1).await;
    let binding = SessionBinding::capture("profile", &hub)
        .await
        .unwrap()
        .unwrap();
    assert!(binding.access_token(None).await.is_err());
    assert!(binding.is_current());
    assert_eq!(
        current_subject("profile", &hub).await.unwrap().as_deref(),
        Some("alice")
    );
    assert_eq!(
        binding.access_token(None).await.unwrap_err(),
        "invalid_grant"
    );
    assert!(!binding.is_current());
    assert!(current_subject("profile", &hub).await.unwrap().is_none());
    assert!(
        native_auth_read("profile".into(), hub)
            .await
            .unwrap()
            .initialized
    );
    server.await.unwrap();
}

#[tokio::test]
async fn no_saved_session_does_not_create_a_login_or_allow_cross_account_access() {
    let hub = unique_hub();
    assert!(
        SessionBinding::capture("profile", "")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        SessionBinding::capture("profile", &hub)
            .await
            .unwrap()
            .is_none()
    );
    install(&hub, "https://issuer.test/token", now_seconds() + 3600).await;
    assert!(access_token("profile", &hub, "bob", None).await.is_err());
}

#[test]
fn refresh_rejects_account_changes_expired_tokens_and_bad_token_types() {
    let now = now_seconds();
    let previous = user("alice", now - 1);
    let configuration = Configuration {
        authority: "https://issuer.test".into(),
        client_id: "client".into(),
        token_endpoint: "https://issuer.test/token".into(),
        ..Default::default()
    };
    for response in [
        json!({"access_token":jwt("bob",now+3600),"token_type":"Bearer"}),
        json!({"access_token":jwt("alice",now-1),"token_type":"Bearer"}),
        json!({"access_token":jwt("alice",now+3600),"token_type":"Basic"}),
        json!({"access_token":jwt("alice",now+3600),"token_type":"Bearer","refresh_token":""}),
    ] {
        assert!(refreshed_user(&previous, response, &configuration, now).is_err());
    }
}

#[test]
fn credential_endpoints_require_tls_and_forbid_url_credentials() {
    for invalid in [
        "http://public.test/token",
        "https://user:pass@issuer.test/token",
        "file:///token",
        "https://issuer.test/token#secret",
        "https://issuer.test\\attacker.test",
    ] {
        assert!(secure_url(invalid).is_err(), "{invalid}");
    }
    assert!(secure_url("http://127.0.0.1:8000/token").is_ok());
    assert!(secure_url("https://issuer.test/token").is_ok());
}

#[tokio::test]
async fn cancelling_a_tool_does_not_discard_the_rotated_grant() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
    let hub = unique_hub();
    install(&hub, &endpoint, 1).await;
    let binding = SessionBinding::capture("profile", &hub)
        .await
        .unwrap()
        .unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let served = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        started_tx.send(()).unwrap();
        release_rx.await.unwrap();
        let body = json!({"access_token":jwt("alice",now_seconds()+3600),"token_type":"Bearer","refresh_token":"rotated-after-cancel"}).to_string();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    });
    let tool = tokio::spawn(async move { binding.access_token(None).await });
    started_rx.await.unwrap();
    tool.abort();
    release_tx.send(()).unwrap();
    served.await.unwrap();
    let snapshot = native_auth_read("profile".into(), hub.clone())
        .await
        .unwrap();
    let user: Value = serde_json::from_str(snapshot.user.as_deref().unwrap()).unwrap();
    assert!(user.get("refresh_token").is_none());
    assert_eq!(
        stored_user(&hub).await["refresh_token"],
        "rotated-after-cancel"
    );
}

#[tokio::test]
async fn profiles_on_the_same_hub_have_independent_login_generations() {
    let hub = unique_hub();
    let first = install(&hub, "https://issuer.test/token", now_seconds() + 3600).await;
    let other = native_auth_read("other-profile".into(), hub.clone())
        .await
        .unwrap();
    write_session(
        "other-profile".into(),
        hub.clone(),
        other.generation,
        Configuration {
            authority: "https://issuer.test".into(),
            client_id: "client".into(),
            token_endpoint: "https://issuer.test/token".into(),
            ..Default::default()
        },
        user("bob", now_seconds() + 3600).to_string(),
    )
    .await
    .unwrap();
    let alice = SessionBinding::capture("profile", &hub)
        .await
        .unwrap()
        .unwrap();
    let bob = SessionBinding::capture("other-profile", &hub)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(alice.subject, "alice");
    assert_eq!(bob.subject, "bob");
    native_auth_clear("profile".into(), hub.clone(), first.generation)
        .await
        .unwrap();
    assert!(!alice.is_current());
    assert!(bob.is_current());
    assert!(bob.access_token(None).await.is_ok());
    assert!(current_subject("profile", &hub).await.unwrap().is_none());
    assert_eq!(
        current_subject("other-profile", &hub)
            .await
            .unwrap()
            .as_deref(),
        Some("bob")
    );
}

#[tokio::test]
async fn rotated_refresh_token_survives_a_temporary_signing_key_outage() {
    let expiry = now_seconds() + 3600;
    let identity=format!("e30.{}.signature",URL_SAFE_NO_PAD.encode(json!({"sub":"alice","iss":"https://issuer.test","aud":"client","exp":expiry,"iat":now_seconds()}).to_string()));
    let (endpoint,_,server)=server(vec![
        (200,json!({"access_token":jwt("alice",expiry),"refresh_token":"rotated","token_type":"Bearer","id_token":identity})),
        (503,json!({"error":"temporarily_unavailable"})),
        (200,json!({"access_token":jwt("alice",expiry),"token_type":"Bearer"})),
    ]).await;
    let hub = unique_hub();
    let snapshot = native_auth_read("profile".into(), hub.clone())
        .await
        .unwrap();
    let mut previous = user("alice", 1);
    previous["id_token"] = json!(identity);
    write_session(
        "profile".into(),
        hub.clone(),
        snapshot.generation,
        Configuration {
            authority: "https://issuer.test".into(),
            client_id: "client".into(),
            token_endpoint: endpoint.clone(),
            jwks_uri: Some(endpoint),
            ..Default::default()
        },
        previous.to_string(),
    )
    .await
    .unwrap();
    let binding = SessionBinding::capture("profile", &hub)
        .await
        .unwrap()
        .unwrap();
    assert!(binding.access_token(None).await.is_err());
    let saved = native_auth_read("profile".into(), hub.clone())
        .await
        .unwrap();
    let saved: Value = serde_json::from_str(saved.user.as_deref().unwrap()).unwrap();
    assert!(saved.get("refresh_token").is_none());
    assert_eq!(stored_user(&hub).await["refresh_token"], "rotated");
    assert_eq!(saved["access_token"], previous["access_token"]);
    assert!(binding.access_token(None).await.is_ok());
    let requests = server.await.unwrap();
    assert!(requests[2].contains("refresh_token=rotated"));
}

#[tokio::test]
async fn malformed_rotated_identity_revokes_the_previous_grant() {
    let (endpoint, _, server) = server(vec![(200, json!({"access_token":jwt("bob", now_seconds()+3600),"refresh_token":"rotated","token_type":"Bearer"}))]).await;
    let hub = unique_hub();
    install(&hub, &endpoint, 1).await;
    let binding = SessionBinding::capture("profile", &hub)
        .await
        .unwrap()
        .unwrap();
    assert!(binding.access_token(None).await.is_err());
    assert!(!binding.is_current());
    assert!(current_subject("profile", &hub).await.unwrap().is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn cached_token_selection_does_not_contact_the_issuer_while_offline() {
    let hub = unique_hub();
    install(&hub, "https://issuer.invalid/token", 1).await;
    let binding = SessionBinding::capture("profile", &hub)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        binding.cached_access_token().await.unwrap(),
        jwt("alice", 1)
    );
}

#[tokio::test]
async fn concurrent_profiles_cannot_import_the_same_browser_refresh_owner() {
    let config = Configuration {
        authority: "https://issuer.test".into(),
        client_id: uuid::Uuid::new_v4().to_string(),
        ..Default::default()
    };
    let hub = unique_hub();
    let mut workers = Vec::new();
    for index in 0..16 {
        let config = config.clone();
        let hub = hub.clone();
        workers.push(tokio::spawn(async move {
            let profile = if index % 2 == 0 {
                "first-profile"
            } else {
                "second-profile"
            };
            (
                profile,
                claim_legacy_import(profile, &hub, &config).await.is_ok(),
            )
        }));
    }
    let mut successful = std::collections::HashSet::new();
    for worker in workers {
        let (profile, success) = worker.await.unwrap();
        if success {
            successful.insert(profile);
        }
    }
    assert_eq!(successful.len(), 1);
    let owner = successful.into_iter().next().unwrap();
    assert!(claim_legacy_import(owner, &hub, &config).await.is_ok());
    assert!(
        claim_legacy_import(owner, "https://another-hub.test", &config)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn deleting_an_uninitialized_profile_fences_inflight_imports_and_login_intents() {
    let hub = unique_hub();
    let before = native_auth_read("profile".into(), hub.clone())
        .await
        .unwrap();
    assert!(!before.initialized);
    invalidate_session("profile", &hub).await.unwrap();
    let after = native_auth_read("profile".into(), hub.clone())
        .await
        .unwrap();
    assert!(after.initialized);
    assert!(after.user.is_none());
    assert_ne!(after.generation, before.generation);
    let configuration = Configuration {
        authority: "https://issuer.test".into(),
        client_id: "client".into(),
        token_endpoint: "https://issuer.test/token".into(),
        ..Default::default()
    };
    for import_only in [false, true] {
        assert!(
            commit_session(
                "profile".into(),
                hub.clone(),
                before.generation.clone(),
                configuration.clone(),
                user("alice", now_seconds() + 3600).to_string(),
                import_only
            )
            .await
            .is_err()
        );
    }
}
